//! # Liquidation (contract owner only)
//!
//! Liquidates a borrower whose LTV is at or above the liquidation rate. The liquidator (owner)
//! repays debt and chooses which collateral to seize. The **market value** (price × amount, no
//! haircut) of seized collateral must be 100% to `liquidation_bonus_rate` of the repay value
//! (e.g. 1.02 = 2% cap; ensures liquidator profit does not exceed the intended bonus).
//!
//! **Flow (see numbered sections in `liquidate`):** auth → debt/collateral checks → liquidatable →
//! minimum repay (USD → lending units) → sent funds and scaled repay → collateral value band and
//! per-asset checks → dry-run post-seizure / bad-debt → persist reserve and collateral → response
//! (collateral send + attrs) → refund excess lending.
//!
//! **Bad debt:** `bad_debt_loss_allocation` on contract state chooses **deferred** (`deficit_underlying`)
//! vs **immediate** (pro-rata `liquidity_index` haircut in the same tx; see `apply_pro_rata_liquidity_index_haircut`).

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_BAD_DEBT_LOSS_ALLOCATION,
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON,
    ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_LIQUIDATOR, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, not_found, ContractError};
use crate::model::health::BorrowerHealthV1;
use crate::model::BadDebtLossAllocation;
use crate::model::BorrowerCollateralV1;
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_scaled_borrow, set_borrower_collateral,
    set_reserve_state_v1, set_scaled_borrow, subtract_total_collateral,
};
use crate::utils::{
    apply_pro_rata_liquidity_index_haircut, calculate_borrow_value_usd,
    calculate_total_collateral_value_usd, decimal256_ceil_to_u128, get_asset_prices_for_borrower,
    get_borrower_health, scaled_to_underlying_borrow, uint128_to_decimal256,
    underlying_to_scaled_borrow, update_reserve_indexes, validate_single_coin_denom, WithRates,
};
use cosmwasm_std::{
    ensure, BankMsg, Coin, Decimal256, DepsMut, Env, MessageInfo, Response, Uint128,
};
use democratized_prime_lib::common::assert_owner;
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "liquidate";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may liquidate";

/// Liquidate a borrower whose LTV ≥ liquidation_rate. Contract owner only. Repay debt from funds and seize
/// collateral per `collateral_to_seize`; value must be 100%–liquidation_bonus_rate of repay. See module doc for flow.
pub fn liquidate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
    collateral_to_seize: &BTreeMap<String, Uint128>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;

    // ---------- 1. Auth and borrower identity ----------
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    let borrower_addr = deps.api.addr_validate(borrower.trim())?;
    let borrower_key = borrower_addr.as_str();

    // ---------- 2. Borrower must have debt and collateral ----------
    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    let scaled_debt = get_scaled_borrow(deps.storage, borrower_key)?;
    ensure!(
        scaled_debt > 0,
        illegal_argument(
            "Borrower has no debt (no scaled borrow on file; may have repaid in full)",
        )
    );
    let debt_underlying = scaled_to_underlying_borrow(scaled_debt, reserve.borrow_index)?;
    ensure!(
        debt_underlying > 0,
        illegal_argument("Borrower has no debt (scaled borrow rounds to zero underlying; dust)",)
    );
    let borrower_collateral = get_borrower_collateral(deps.storage, borrower_key)?;
    ensure!(
        !borrower_collateral.amounts.is_empty(),
        illegal_argument("Borrower has no collateral")
    );

    // ---------- 3. Must be liquidatable (LTV >= liquidation_rate) ----------
    let asset_prices = get_asset_prices_for_borrower(
        &deps.querier,
        &env.block.time,
        &contract,
        &borrower_collateral,
    )?;
    let (health, _ltv) = get_borrower_health(
        &contract,
        &contract.supported_collateral_assets,
        &asset_prices,
        &borrower_collateral,
        Uint128::from(debt_underlying),
    )?;
    ensure!(
        health == BorrowerHealthV1::Liquidatable,
        illegal_argument("Borrower is not liquidatable (LTV below liquidation rate)")
    );

    // ---------- 4. Minimum repay (USD): healthy target or collateral cap; then lending base units ----------
    // Target healthy LTV: after repay r and seizing bonus*r of collateral, (D - r) / (C - bonus*r) = margin_rate.
    // Solving: r = (D - margin_rate*C) / (1 - bonus*margin_rate). Cap at debt, at zero, and at total collateral USD.
    let debt_value_usd = calculate_borrow_value_usd(
        Uint128::from(debt_underlying),
        &contract.lending_denom.name,
        &asset_prices,
    )?;
    let collateral_value_usd = calculate_total_collateral_value_usd(
        &borrower_collateral,
        &asset_prices,
        &contract.supported_collateral_assets,
    )?;

    let one = Decimal256::one();
    let bonus = contract.liquidation_bonus_rate;
    let bonus_times_margin = bonus
        .checked_mul(contract.margin_rate)
        .map_err(|_| illegal_state("liquidation_bonus_rate * margin_rate overflow"))?;
    ensure!(
        bonus_times_margin < one,
        illegal_state(
            "Liquidation impossible: liquidation_bonus_rate * margin_rate must be < 1 \
             (current config would make denominator 1 - bonus*margin_rate non-positive)"
        )
    );
    let denominator = one.checked_sub(bonus_times_margin).map_err(|_| {
        illegal_state(
            "Liquidation denominator underflow (1 - liquidation_bonus_rate*margin_rate); \
             config invalid",
        )
    })?;
    ensure!(
        !denominator.is_zero(),
        illegal_state("Liquidation denominator zero (1 - liquidation_bonus_rate*margin_rate)")
    );
    // Within step 4: numerator = debt - margin*C. With LTV >= liquidation_rate, exact math gives numerator >= 0.
    // Decimal256 rounding can make margin*C >= debt; saturating_sub yields 0 so we don't revert.
    let numerator =
        debt_value_usd.saturating_sub(contract.margin_rate.checked_mul(collateral_value_usd)?);
    let min_repay_value_to_healthy_usd = numerator.checked_div(denominator)?;
    // Clamp: min_repay must be in [0, debt_value_usd] (no negative, can't require more than full debt).
    let min_repay_value_to_healthy_usd = if min_repay_value_to_healthy_usd <= Decimal256::zero() {
        Decimal256::zero()
    } else if min_repay_value_to_healthy_usd > debt_value_usd {
        debt_value_usd
    } else {
        min_repay_value_to_healthy_usd
    };

    // If total collateral is less than the repay needed to reach healthy, cap min repay at collateral
    // value so we can still partially liquidate (take all collateral, repay up to that value).
    let min_repay_value_usd = if min_repay_value_to_healthy_usd > collateral_value_usd {
        collateral_value_usd
    } else {
        min_repay_value_to_healthy_usd
    };

    let price_lending = asset_prices
        .get(&contract.lending_denom.name)
        .ok_or_else(|| {
            not_found(format!(
                "Price of lending denom is missing: {}",
                contract.lending_denom.name
            ))
        })?
        .price_usd;
    ensure!(
        !price_lending.is_zero(),
        illegal_state("Lending denom price is zero")
    );
    // Still step 4: guard above avoids divide-by-zero in checked_div(price_lending).
    let min_repay_lending =
        decimal256_ceil_to_u128(min_repay_value_usd.checked_div(price_lending)?)
            .ok_or_else(|| illegal_state("Min repay amount overflow"))?;
    // Clamp min repay to at least 1 base unit (min_repay_value_usd can be 0 when numerator saturates;
    // decimal256_ceil_to_u128(0) returns 0).
    let min_repay_lending = min_repay_lending.max(1);

    // ---------- 5. Attached lending funds; actual repay and scaled repay ----------
    let sent = validate_single_coin_denom(
        &info,
        &contract.lending_denom,
        Uint128::from(min_repay_lending),
    )?;
    let sent_u128 = sent.u128();
    let actual_repay_underlying = sent_u128.min(debt_underlying);
    // Full repay: use scaled_debt directly to avoid double-floor dust (same as repay.rs).
    let scaled_repay = if actual_repay_underlying >= debt_underlying {
        scaled_debt
    } else {
        underlying_to_scaled_borrow(actual_repay_underlying, reserve.borrow_index)?
    };
    let new_scaled_debt = scaled_debt
        .checked_sub(scaled_repay)
        .ok_or_else(|| illegal_state("scaled debt underflow"))?;

    // ---------- 6. Collateral to seize: non-empty map; per-asset support, balances; USD band vs repay ----------
    let actual_repay_value_usd =
        price_lending.checked_mul(uint128_to_decimal256(actual_repay_underlying))?;
    let min_collateral_value_required = actual_repay_value_usd; // 100% of repay value
    let max_collateral_value_allowed = actual_repay_value_usd.checked_mul(bonus)?;
    ensure!(
        !collateral_to_seize.is_empty(),
        illegal_argument("collateral_to_seize must specify at least one asset and amount",)
    );

    let supported_ids: HashSet<_> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| a.asset_id.as_str())
        .collect();

    // Value each requested seizure at market price (price × amount). Band is on market value so the
    // liquidation bonus cap applies to economic seize size (not haircutted collateral value).
    let mut seized_value_usd = Decimal256::zero();
    for (asset_id, seize_amount) in collateral_to_seize {
        if seize_amount.is_zero() {
            continue;
        }
        ensure!(
            supported_ids.contains(asset_id.as_str()),
            illegal_argument(format!(
                "Unsupported collateral asset in collateral_to_seize: {}",
                asset_id
            ))
        );
        let borrower_has = *borrower_collateral
            .amounts
            .get(asset_id.as_str())
            .unwrap_or(&0);
        ensure!(
            borrower_has >= seize_amount.u128(),
            illegal_argument(format!(
                "Borrower has insufficient collateral for {}: have {}, requested {}",
                asset_id, borrower_has, seize_amount
            ))
        );
        let price = asset_prices
            .get(asset_id)
            .ok_or_else(|| not_found(format!("Price of asset: {}", asset_id)))?
            .price_usd;
        let value = price.checked_mul(uint128_to_decimal256(seize_amount.u128()))?;
        seized_value_usd = seized_value_usd.checked_add(value)?;
    }

    ensure!(
        seized_value_usd >= min_collateral_value_required,
        illegal_argument(format!(
            "Collateral to seize value {} is below required 100% of repay value {}",
            seized_value_usd, min_collateral_value_required
        ))
    );
    ensure!(
        seized_value_usd <= max_collateral_value_allowed,
        illegal_argument(format!(
            "Collateral to seize value {} exceeds allowed maximum (liquidation_bonus_rate) of repay value {} (borrower protection)",
            seized_value_usd, max_collateral_value_allowed
        ))
    );

    // ---------- 7. Seizure list; dry-run post-seizure collateral; bad-debt flag ----------
    let to_seize: Vec<(String, u128)> = collateral_to_seize
        .iter()
        .filter(|(_, amt)| !amt.is_zero())
        .map(|(id, amt)| (id.clone(), amt.u128()))
        .collect();
    ensure!(
        !to_seize.is_empty(),
        illegal_argument(
            "collateral_to_seize must contain at least one asset with positive amount",
        )
    );

    // Dry-run remaining borrower collateral after this seizure (detect bad debt before persisting).
    let mut new_amounts = borrower_collateral.amounts.clone();
    for (asset_id, seize_amt) in &to_seize {
        let cur = *new_amounts.get(asset_id).unwrap_or(&0);
        let remaining = cur
            .checked_sub(*seize_amt)
            .ok_or_else(|| illegal_state("collateral underflow"))?;
        if remaining > 0 {
            new_amounts.insert(asset_id.clone(), remaining);
        } else {
            new_amounts.remove(asset_id);
        }
    }
    let bad_debt = new_amounts.is_empty() && new_scaled_debt > 0;
    let bad_debt_underlying_amt = if bad_debt {
        scaled_to_underlying_borrow(new_scaled_debt, reserve.borrow_index)?
    } else {
        0u128
    };

    // ---------- 8. Persist reserve aggregates, borrower scaled debt, protocol collateral totals, borrower map ----------
    if bad_debt {
        let scaled_writeoff = new_scaled_debt;
        set_scaled_borrow(deps.storage, borrower_key, 0).map_err(ContractError::Std)?;
        let total_sub = scaled_repay
            .checked_add(scaled_writeoff)
            .ok_or_else(|| illegal_state("scaled repay + writeoff overflow"))?;
        reserve.total_scaled_borrow = reserve
            .total_scaled_borrow
            .checked_sub(total_sub)
            .ok_or_else(|| illegal_state("total_scaled_borrow underflow"))?;
        match contract.bad_debt_loss_allocation {
            BadDebtLossAllocation::ImmediateLiquidityIndexHaircut => {
                apply_pro_rata_liquidity_index_haircut(&mut reserve, bad_debt_underlying_amt)?;
            }
            BadDebtLossAllocation::DeferredToDeficit => {
                reserve.deficit_underlying = reserve
                    .deficit_underlying
                    .checked_add(bad_debt_underlying_amt)
                    .ok_or_else(|| illegal_state("deficit_underlying overflow"))?;
            }
        }
    } else {
        set_scaled_borrow(deps.storage, borrower_key, new_scaled_debt)
            .map_err(ContractError::Std)?;
        reserve.total_scaled_borrow = reserve
            .total_scaled_borrow
            .checked_sub(scaled_repay)
            .ok_or_else(|| illegal_state("total_scaled_borrow underflow"))?;
    }
    set_reserve_state_v1(deps.storage, &reserve)?;

    let mut send_coins: Vec<Coin> = Vec::with_capacity(to_seize.len());
    for (asset_id, seize_amt) in &to_seize {
        subtract_total_collateral(deps.storage, asset_id, *seize_amt)?;
        send_coins.push(Coin {
            denom: asset_id.clone(),
            amount: Uint128::from(*seize_amt),
        });
    }
    set_borrower_collateral(
        deps.storage,
        borrower_key,
        &BorrowerCollateralV1 {
            amounts: new_amounts,
        },
    )?;

    let collateral_json: BTreeMap<String, String> = send_coins
        .iter()
        .map(|c| (c.denom.clone(), c.amount.to_string()))
        .collect();

    // ---------- 9. Response: collateral BankMsg, standard attributes, optional bad-debt attributes ----------
    let mut res = Response::new()
        .add_message(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: send_coins.clone(),
        })
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_LIQUIDATOR, info.sender.as_str())
        .add_attribute(ATTRIBUTE_BORROWER, borrower_key)
        .add_attribute(ATTRIBUTE_AMOUNT, actual_repay_underlying.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_repay.to_string())
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        );
    if bad_debt {
        res = res
            .add_attribute(
                ATTRIBUTE_BAD_DEBT_UNDERLYING,
                bad_debt_underlying_amt.to_string(),
            )
            .add_attribute(
                ATTRIBUTE_DEFICIT_UNDERLYING,
                reserve.deficit_underlying.to_string(),
            )
            .add_attribute(
                ATTRIBUTE_BAD_DEBT_LOSS_ALLOCATION,
                contract.bad_debt_loss_allocation.as_str(),
            );
    }

    // ---------- 10. Refund excess lending (sent amount above applied repay) ----------
    if sent_u128 > actual_repay_underlying {
        let excess = sent_u128 - actual_repay_underlying;
        res = res.add_message(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: vec![Coin {
                denom: contract.lending_denom.name.clone(),
                amount: Uint128::from(excess),
            }],
        });
    }

    res.attach_rates(&reserve, &contract.rate_params)
}
