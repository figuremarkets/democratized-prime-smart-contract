//! # Liquidation
//!
//! Liquidates a borrower whose LTV is at or above the liquidation rate. Auth follows
//! [`crate::model::LiquidationAccess`]: owner-only by default, or any sender when the
//! custodian has set **permissionless** and the position is liquidatable even after
//! counting unpriceable holdings at their retained last-known (too-old, non-zero) quotes.
//! A dropped feed with **no** stored quote, or a last-known that would pull LTV back
//! below the liquidation rate, still requires the owner — so a stranger cannot seize
//! the priced remainder of a solvent mixed bag. Dust of a dead feed does not move LTV
//! and does not disable permissionless. The liquidator repays debt and chooses which
//! collateral to seize. The **market value**
//! (`display_price_usd × amount / 10^precision`, no haircut) of seized collateral must be
//! 100% to `liquidation_bonus_rate` of the repay value
//! (e.g. 1.02 = 2% cap; ensures liquidator profit does not exceed the intended bonus).
//! A remainder worth **$0** after the seizure (empty map, or leftover with no haircutted USD)
//! waives only the 100% floor; the bonus cap still applies, so a 1-atom repay can empty only a
//! dust bag. Residual debt against that remainder is booked in the same transaction via
//! `bad_debt_loss_allocation`.
//!
//! **How much must be repaid:** there is no closed-form minimum. Actual repay is
//! `min(sent, ceil(scaled × borrow_index))`. The repay plus seizure must leave the borrower
//! at or below `margin_rate`, which is checked against the real post-state (`new_amounts` and
//! `new_scaled_debt`) rather than predicted up front. Full repayment, a full close, and a
//! remainder whose haircutted USD is zero (unpriceable leftover, or priceable leftover that
//! truncates to $0) are exempt from that health check — residual debt against a zero-value
//! bag is booked as bad debt in the same tx. The attached amount must reduce scaled debt;
//! sub-index repayments are rejected before collateral can be seized. Cancelling all scaled
//! debt requires the ceiled payoff, whether or not the collateral map empties.
//!
//! An earlier formula, `r = (D - margin_rate*C) / (1 - liquidation_bonus_rate*margin_rate)`,
//! mixed units: `C` is haircutted collateral USD, but the seizure band bounds the seizure by
//! its *market* (un-haircutted) value, so a $1 repay only removes `haircut * bonus * $1` of
//! health-relevant collateral, not `bonus * $1`. That overstated the requirement by roughly
//! `1/haircut` and forced over-liquidation: the borrower could not be brought to exactly
//! `margin_rate`, only far past it. No corrected closed form exists either, because the
//! liquidator picks which assets to seize and haircuts are per-asset. Step 7b validates the
//! real post-state instead.
//!
//! Collateral with **no stored** oracle price, a **zero** stored price, or last-known older than
//! `max_liquidation_staleness_seconds`, is valued at zero for LTV and cannot
//! be seized. A **stale** stored price still within that bound is used as last-known (not fatal)
//! so liquidations are not frozen by a paused feed. The lending denom must have a stored price
//! within the same bound.
//!
//! **Flow (see numbered sections in `liquidate`):** auth → debt/collateral checks → prices →
//! liquidatable → mixed-bag owner gate (load-bearing unpriceable only) → lending price →
//! sent funds and scaled repay → per-asset checks and dry-run post-seizure → value band
//! (100% floor waived when the remainder is worth $0) → post-state health → persist reserve and collateral →
//! response (collateral send + attrs) → refund excess lending.
//!
//! **Bad debt:** `bad_debt_loss_allocation` on contract state chooses **deferred** (`deficit_underlying`)
//! vs **immediate** (pro-rata `liquidity_index` haircut in the same tx; see `apply_pro_rata_liquidity_index_haircut`).

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_BAD_DEBT_LOSS_ALLOCATION,
    ATTRIBUTE_BAD_DEBT_UNDERLYING, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON,
    ATTRIBUTE_DEFICIT_UNDERLYING, ATTRIBUTE_LIQUIDATION_ACCESS, ATTRIBUTE_LIQUIDATOR,
    ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, not_found, ContractError};
use crate::model::health::BorrowerHealthV1;
use crate::model::BorrowerCollateralV1;
use crate::model::{BadDebtLossAllocation, ContractStateV1, LiquidationAccess};
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_scaled_borrow, set_borrower_collateral,
    set_reserve_state_v1, set_scaled_borrow, subtract_total_collateral,
};
use crate::utils::{
    apply_pro_rata_liquidity_index_haircut, calculate_total_collateral_value_usd,
    format_as_percent_string, get_asset_prices_for_liquidation, get_borrower_health,
    scaled_to_underlying_borrow, scaled_to_underlying_borrow_ceil, underlying_to_scaled_borrow,
    update_reserve_indexes, validate_single_coin_denom, LiquidationPrices, WithRates,
};
use cosmwasm_std::{
    ensure, BankMsg, Coin, Decimal256, DepsMut, Env, MessageInfo, Response, Uint128,
};
use democratized_prime_lib::common::assert_owner;
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "liquidate";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may liquidate";
pub const ASSERT_OWNER_UNPRICEABLE_ERR: &str =
    "Only the contract owner may liquidate when unpriceable collateral is load-bearing";

/// Liquidate a borrower whose LTV ≥ liquidation_rate. Auth follows [`LiquidationAccess`].
/// Permissionless still requires the owner when unpriceable collateral is load-bearing
/// (counting last-known of dropped feeds would make the position not liquidatable, or a
/// dropped feed has no stored quote).
/// Repay debt from funds and seize collateral per `collateral_to_seize`; market value must be
/// 100%–liquidation_bonus_rate of repay, except a $0 remainder waives the 100% floor (bonus cap
/// still applies). The resulting post-state must be at or below `margin_rate` — there is no
/// precomputed minimum repay. Residual debt against an empty or zero-value remainder is booked
/// as bad debt. See module doc for flow.
pub fn liquidate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    borrower: String,
    collateral_to_seize: &BTreeMap<String, Uint128>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;

    // ---------- 1. Auth and borrower identity ----------
    if matches!(contract.liquidation_access, LiquidationAccess::OwnerOnly) {
        assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    }
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
    let debt_payoff = scaled_to_underlying_borrow_ceil(scaled_debt, reserve.borrow_index)?;
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
    let quoted = get_asset_prices_for_liquidation(
        &deps.querier,
        &env.block.time,
        &contract,
        &borrower_collateral,
    )?;
    let asset_prices = &quoted.prices;
    let has_priceable_collateral = borrower_collateral
        .amounts
        .iter()
        .any(|(id, amt)| *amt > 0 && !quoted.unpriceable.contains(id));
    ensure!(
        has_priceable_collateral,
        illegal_argument(
            "No priceable collateral (all held assets have no stored, zero, or over-stale oracle price)",
        )
    );
    let (health, _ltv) = get_borrower_health(
        &contract,
        &contract.supported_collateral_assets,
        asset_prices,
        &borrower_collateral,
        Uint128::from(debt_underlying),
    )?;
    ensure!(
        health == BorrowerHealthV1::Liquidatable,
        illegal_argument("Borrower is not liquidatable (LTV below liquidation rate)")
    );
    // Unpriceable holdings are omitted from the acted-on USD, which inflates LTV. Require
    // the owner when counting retained last-knowns would make the position not liquidatable
    // (or a dropped feed has no stored quote). Dust of a dead feed does not trip this.
    if matches!(
        contract.liquidation_access,
        LiquidationAccess::Permissionless
    ) && unpriceable_is_load_bearing(&contract, &quoted, &borrower_collateral, debt_underlying)?
    {
        assert_owner(deps.storage, &info.sender, ASSERT_OWNER_UNPRICEABLE_ERR)?;
    }

    // ---------- 4. Lending denom price (repay valuation for the seizure band) ----------
    let price_lending = asset_prices
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
    let bonus = contract.liquidation_bonus_rate;

    // ---------- 5. Attached lending funds; actual repay and scaled repay ----------
    // Only a non-zero amount is required; how much is *enough* is decided by post-state health.
    let sent = validate_single_coin_denom(&info, &contract.lending_denom, Uint128::one())?;
    let sent_u128 = sent.u128();
    // LTV/health use floor debt; cancelling all scaled units requires and collects ceil(s · bi).
    let actual_repay_underlying = sent_u128.min(debt_payoff);
    let scaled_repay = if actual_repay_underlying >= debt_payoff {
        scaled_debt
    } else {
        underlying_to_scaled_borrow(actual_repay_underlying, reserve.borrow_index)?
    };
    ensure!(
        scaled_repay > 0,
        illegal_argument("Repay amount too small to reduce debt")
    );
    let new_scaled_debt = scaled_debt
        .checked_sub(scaled_repay)
        .ok_or_else(|| illegal_state("scaled debt underflow"))?;

    // ---------- 6. Per-asset support/balances; seizure list; dry-run post-seizure ----------
    let actual_repay_value_usd = price_lending.value_usd(actual_repay_underlying)?;
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

    // Value each requested seizure at market (display × amount / 10^precision). Band is on market
    // value so the liquidation bonus cap applies to economic seize size (not haircutted collateral).
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
        ensure!(
            !quoted.unpriceable.contains(asset_id),
            illegal_argument(format!(
                "Cannot seize unpriceable collateral (no stored, zero, or over-stale oracle price): {}",
                asset_id
            ))
        );
        let price = asset_prices
            .get(asset_id)
            .ok_or_else(|| not_found(format!("Price of asset: {}", asset_id)))?;
        let value = price.value_usd(seize_amount.u128())?;
        seized_value_usd = seized_value_usd.checked_add(value)?;
    }

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

    // Dry-run remaining borrower collateral after this seizure (detect a $0 remainder
    // before the value band, so the 100% floor can be waived when nothing of value is left).
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
    let post_collateral = BorrowerCollateralV1 {
        amounts: new_amounts,
    };
    let post_collateral_value_usd = calculate_total_collateral_value_usd(
        &post_collateral,
        asset_prices,
        &contract.supported_collateral_assets,
    )?;

    // ---------- 7. USD band vs repay (100% floor waived when the remainder is worth $0) ----------
    ensure!(
        post_collateral_value_usd.is_zero() || seized_value_usd >= min_collateral_value_required,
        illegal_argument(format!(
            "Collateral to seize value {} is below required 100% of repay value {} \
             (waived only when the seizure leaves a remainder worth nothing)",
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

    // ---------- 7b. Post-state health: the seizure must actually restore the position ----------
    // Replaces the old closed-form minimum repay (see module doc). Checking the real post-state
    // is haircut-correct by construction, so a liquidator can now bring the borrower to exactly
    // margin_rate, and it handles multi-asset seizures whose haircuts differ. Exempt: a full
    // close (no collateral left to be healthy about), a full repayment (no debt left), and a
    // remainder whose haircutted USD is zero. `calculate_ltv` returns the sentinel 1 on a
    // zero-value bag, which would classify Liquidatable and strand the position — unpriceable
    // leftover cannot be seized, and a haircut that truncates remaining units to $0 contributes
    // nothing to LTV. Residual debt against that bag is booked as bad debt below (same as a
    // full close), so it does not sit on the books unallocated. Not exploitable: the band still
    // charges ~full market value for the priced collateral that is taken.

    // A remainder worth nothing routes residual scaled debt to the bad-debt path below. If the
    // liquidator supplied the floored debt but not the ceiled payoff, that residual is a one-unit
    // rounding artefact, not insolvency — reject rather than book it as a loss. This must key off
    // the same condition as `bad_debt`; an empty map is only one way to reach $0.
    ensure!(
        !(post_collateral_value_usd.is_zero()
            && sent_u128 >= debt_underlying
            && sent_u128 < debt_payoff),
        illegal_argument(
            "Cancelling all scaled debt against a zero-value remainder requires the ceiled payoff amount",
        )
    );

    if new_scaled_debt > 0 && !post_collateral_value_usd.is_zero() {
        let post_debt_underlying =
            scaled_to_underlying_borrow(new_scaled_debt, reserve.borrow_index)?;
        let (post_health, post_ltv) = get_borrower_health(
            &contract,
            &contract.supported_collateral_assets,
            asset_prices,
            &post_collateral,
            Uint128::from(post_debt_underlying),
        )?;
        ensure!(
            post_health == BorrowerHealthV1::Healthy,
            illegal_argument(format!(
                "Liquidation would leave the borrower at LTV {} ({:?}), above margin_rate \
                 {}: repay more, or seize more collateral within the bonus cap \
                 (a seizure that empties the borrower collateral map is exempt)",
                format_as_percent_string(post_ltv)?,
                post_health,
                format_as_percent_string(contract.margin_rate)?
            ))
        );
    }

    // Empty map or remaining collateral with no haircutted USD: leftover scaled debt is a
    // loss to book, not a live borrow. An empty map is the usual case; a mixed bag that
    // clears every priceable unit (or a remainder that haircuts to $0) hits the same path.
    let bad_debt = new_scaled_debt > 0 && post_collateral_value_usd.is_zero();
    let bad_debt_underlying_amt = if bad_debt {
        // Cover the aggregate floor drop from cancelling `new_scaled_debt`. `floor(s' · bi)` can
        // be 1 short of `floor(T · bi) − floor((T − s') · bi)`; ceil is at most 1 over exact and
        // keeps implied cash conservative. Immediate haircut still requires the amount `< L`.
        scaled_to_underlying_borrow_ceil(new_scaled_debt, reserve.borrow_index)?
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
    set_borrower_collateral(deps.storage, borrower_key, &post_collateral)?;

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
        )
        .add_attribute(
            ATTRIBUTE_LIQUIDATION_ACCESS,
            contract.liquidation_access.as_str(),
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

/// True when omitting unpriceable holdings is what made the position look liquidatable.
/// Retained last-known quotes are an auth input only — never the acted-on health or seizure
/// band. A missing or zero stored quote cannot be counted, so that holding is treated as load-bearing.
fn unpriceable_is_load_bearing(
    contract: &ContractStateV1,
    quoted: &LiquidationPrices,
    collateral: &BorrowerCollateralV1,
    debt_underlying: u128,
) -> Result<bool, ContractError> {
    if quoted.unpriceable.is_empty() {
        return Ok(false);
    }
    let missing_last_known = quoted.unpriceable.iter().any(|id| {
        collateral.amounts.get(id).copied().unwrap_or(0) > 0
            && !quoted.unpriceable_last_known.contains_key(id)
    });
    if missing_last_known {
        return Ok(true);
    }
    let (health, _) = get_borrower_health(
        contract,
        &contract.supported_collateral_assets,
        &quoted.prices_with_unpriceable_last_known(),
        collateral,
        Uint128::from(debt_underlying),
    )?;
    Ok(health != BorrowerHealthV1::Liquidatable)
}
