//! # Write-off (contract owner only)
//!
//! Closes leftover debt when **priced** collateral **market** value is worth less than one
//! lending-denom base unit. Liquidate cannot close those positions: min repay is at least one
//! lending atom and seized market value must be ≥ 100% of repay, so a bag worth less than one
//! atom never satisfies the band.
//!
//! Seizes only **priced** dust (sent to the owner). Missing, zero, or last-known-beyond-bound
//! collateral is unpriceable: it contributes $0 to the dust bar (so a delisted-feed position
//! can still be written off) but is **not seized**. Those balances stay on the borrower map;
//! after scaled debt is zeroed, the borrower can [`crate::execute::remove_collateral`] without
//! a health check. Unpriceable $0 is an accounting convention, not a market fact — a dark
//! feed must not become an owner sweep.
//!
//! The dust bar uses `display_price_usd × amount / 10^precision` of priced collateral (not
//! the deprecated scaled `price_usd`). Priced dust is worth less than one lending atom.
//!
//! **Optional lending funds:** applied as repay first. Residual scaled debt uses
//! [`crate::model::BadDebtLossAllocation`]. Sending nothing socializes the whole loan;
//! sending the full debt covers it so the pool records nothing.
//!
//! Under [`crate::model::BadDebtLossAllocation::ImmediateLiquidityIndexHaircut`], a residual
//! loss that would floor `liquidity_index` to zero reverts (same ensure as SocializeDeficit).
//! Positions whose **priced** collateral is still worth at least one lending unit use Liquidate.
//!
//! Prices: last-known within `max_liquidation_staleness_seconds`, same as Liquidate. The
//! lending denom must have a usable stored price (the dust bar is undefined without it).
//!
//! **Flow (see numbered sections in `write_off`):** owner → debt → dust gate → optional repay →
//! persist (zero debt, book residual) → send priced dust → leave unpriceable → refund excess.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_BAD_DEBT_LOSS_ALLOCATION,
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON,
    ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_SCALED_AMOUNT, ATTRIBUTE_SENDER,
    ATTRIBUTE_UNPRICEABLE_JSON,
};
use crate::model::error::{illegal_argument, illegal_state, not_found, ContractError};
use crate::model::BadDebtLossAllocation;
use crate::model::BorrowerCollateralV1;
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_scaled_borrow, set_borrower_collateral,
    set_reserve_state_v1, set_scaled_borrow, subtract_total_collateral,
};
use crate::utils::{
    apply_pro_rata_liquidity_index_haircut, calculate_total_collateral_market_value_usd,
    get_asset_prices_for_liquidation, scaled_to_underlying_borrow, underlying_to_scaled_borrow,
    update_reserve_indexes, validate_single_coin_denom, WithRates, ZeroPricePolicy,
};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};
use democratized_prime_lib::common::assert_owner;

pub const ACTION: &str = "write_off";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may write off bad debt";

/// Owner-only: write off residual scaled debt; seize priced dust only.
pub fn write_off(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;

    // ---------- 1. Owner only (not custodian) ----------
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    let borrower_addr = deps.api.addr_validate(borrower.trim())?;
    let borrower_key = borrower_addr.as_str();

    // ---------- 2. Borrower must have positive underlying debt ----------
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

    // ---------- 3. Dust gate: priced collateral market value < one lending atom ----------
    // Market value (no haircut). Lending price must be usable so the bar is defined.
    // At or above one atom, Liquidate's 100% band can succeed — refuse here.
    // Unpriceable collateral contributes $0 (TreatAsWorthless) so a delisted feed can still
    // be written off, but is not seized (see §6).
    let borrower_collateral = get_borrower_collateral(deps.storage, borrower_key)?;
    let quoted = get_asset_prices_for_liquidation(
        &deps.querier,
        &env.block.time,
        &contract,
        &borrower_collateral,
    )?;
    // Defensive: get_asset_prices_for_liquidation already partitions every collateral key
    // into prices XOR unpriceable. This cannot fire unless that helper's contract changes.
    for (asset_id, amt) in borrower_collateral.amounts.iter() {
        if *amt == 0 {
            continue;
        }
        ensure!(
            quoted.prices.contains_key(asset_id) || quoted.unpriceable.contains(asset_id),
            illegal_state(format!(
                "Collateral {asset_id} is neither priced nor classified unpriceable"
            ))
        );
    }
    let price_lending = quoted
        .prices
        .get(&contract.lending_denom.name)
        .ok_or_else(|| {
            not_found(format!(
                "Price of lending denom is missing: {}",
                contract.lending_denom.name
            ))
        })?;
    ensure!(
        !price_lending.is_zero_price(),
        illegal_state("Lending denom price is zero")
    );

    let market_value_usd = calculate_total_collateral_market_value_usd(
        &borrower_collateral,
        &quoted.prices,
        ZeroPricePolicy::TreatAsWorthless,
    )?;
    let one_lending_unit_usd = price_lending.value_usd(1)?;
    ensure!(
        !one_lending_unit_usd.is_zero(),
        illegal_state("Lending denom price too small to define the dust bar")
    );
    ensure!(
        market_value_usd < one_lending_unit_usd,
        illegal_argument(
            "Collateral market value is at least one lending base unit; use Liquidate"
        )
    );

    // ---------- 4. Optional repay: owner may cover some/all debt so less (or none) is socialized ----------
    // Empty funds → residual = full debt. Partial → book the unpaid slice. Full (sent >= debt) →
    // leftover 0, no deficit/haircut; use scaled_debt directly to avoid floor-division dust.
    let (actual_repay_underlying, sent_u128) = if info.funds.is_empty() {
        (0u128, 0u128)
    } else {
        let sent =
            validate_single_coin_denom(&info, &contract.lending_denom, Uint128::one())?.u128();
        (sent.min(debt_underlying), sent)
    };
    let scaled_repay = if actual_repay_underlying == 0 {
        0u128
    } else if actual_repay_underlying >= debt_underlying {
        scaled_debt
    } else {
        underlying_to_scaled_borrow(actual_repay_underlying, reserve.borrow_index)?
    };
    let new_scaled_debt = scaled_debt
        .checked_sub(scaled_repay)
        .ok_or_else(|| illegal_state("scaled debt underflow"))?;

    let to_seize: Vec<(String, u128)> = borrower_collateral
        .amounts
        .iter()
        .filter(|(id, amt)| **amt > 0 && !quoted.unpriceable.contains(*id))
        .map(|(id, amt)| (id.clone(), *amt))
        .collect();
    let remaining: std::collections::BTreeMap<String, u128> = borrower_collateral
        .amounts
        .iter()
        .filter(|(id, amt)| **amt > 0 && quoted.unpriceable.contains(*id))
        .map(|(id, amt)| (id.clone(), *amt))
        .collect();

    let bad_debt = new_scaled_debt > 0;
    let bad_debt_underlying_amt = if bad_debt {
        scaled_to_underlying_borrow(new_scaled_debt, reserve.borrow_index)?
    } else {
        0u128
    };

    // ---------- 5. Zero the borrower; book leftover (if any) as deficit or liquidity-index haircut ----------
    // Always remove the full scaled_debt from the reserve (repay + write-off). Leftover 0 means
    // owner covered the loan in this tx — skip allocation.
    set_scaled_borrow(deps.storage, borrower_key, 0).map_err(ContractError::Std)?;
    reserve.total_scaled_borrow = reserve
        .total_scaled_borrow
        .checked_sub(scaled_debt)
        .ok_or_else(|| illegal_state("total_scaled_borrow underflow"))?;
    if bad_debt {
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
    }
    set_reserve_state_v1(deps.storage, &reserve)?;

    // ---------- 6. Seize priced dust only; leave unpriceable balances on the borrower ----------
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
        &BorrowerCollateralV1 { amounts: remaining },
    )?;

    // ---------- 7. Response: attrs, collateral send, unpriceable set, bad-debt attrs, refund ----------
    let collateral_json: std::collections::BTreeMap<String, String> = send_coins
        .iter()
        .map(|c| (c.denom.clone(), c.amount.to_string()))
        .collect();
    let mut unpriceable_ids: Vec<String> = quoted.unpriceable.into_iter().collect();
    unpriceable_ids.sort();

    let mut res = Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_SENDER, info.sender.as_str())
        .add_attribute(ATTRIBUTE_BORROWER, borrower_key)
        .add_attribute(ATTRIBUTE_AMOUNT, actual_repay_underlying.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_repay.to_string())
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        )
        .add_attribute(
            ATTRIBUTE_UNPRICEABLE_JSON,
            serde_json::to_string(&unpriceable_ids).unwrap_or_default(),
        );
    if !send_coins.is_empty() {
        res = res.add_message(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: send_coins,
        });
    }
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
