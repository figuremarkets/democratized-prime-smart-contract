use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_BORROWER, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_scaled_borrow, set_reserve_state_v1,
    set_scaled_borrow,
};
use crate::utils::{
    get_asset_prices_for_borrower, get_borrower_health, reserve_totals_and_cash_u128,
    scaled_to_underlying_borrow, underlying_to_scaled_borrow_ceil, update_reserve_indexes,
    validate_borrower_attrs, validate_borrower_is_healthy, WithRates,
};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};

pub const ACTION: &str = "borrow";

/// Borrow: user receives lending-denom coins up to the requested amount. Requires collateral,
/// borrower attributes, and LTV (existing + new debt vs collateral value) below margin_rate.
/// Amount is capped by pool cash (`reserve_totals_and_cash_u128`: floored lend minus borrow minus
/// deficit, **saturating at zero**). We accrue interest, then add
/// the borrow to the user's scaled debt and send the coins.
pub fn borrow(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, ContractError> {
    ensure!(
        !amount.is_zero(),
        illegal_argument("Borrow amount must be positive")
    );
    let contract = get_contract_state_v1(deps.storage)?;
    ensure!(
        amount >= contract.min_borrow,
        illegal_argument(format!(
            "Borrow amount must be at least {} (min_borrow)",
            contract.min_borrow
        ))
    );
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    validate_borrower_attrs(
        &deps.querier,
        info.sender.as_str(),
        &contract.borrower_required_attrs,
    )?;

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;

    let (_total_liquidity_u128, _total_borrow_u128, cash) = reserve_totals_and_cash_u128(&reserve)?;
    ensure!(
        amount.u128() <= cash,
        illegal_argument(format!(
            "Insufficient liquidity: borrow amount {} exceeds available cash {}",
            amount, cash
        ))
    );

    let borrower_collateral = get_borrower_collateral(deps.storage, info.sender.as_str())?;
    ensure!(
        !borrower_collateral.amounts.is_empty(),
        illegal_argument("Cannot borrow without collateral; add collateral first")
    );

    let current_scaled = get_scaled_borrow(deps.storage, info.sender.as_str())?;
    let scaled_delta = underlying_to_scaled_borrow_ceil(amount.u128(), reserve.borrow_index)?;
    let new_scaled = current_scaled.checked_add(scaled_delta).ok_or_else(|| {
        illegal_state("overflow: borrower scaled debt (current_scaled + scaled_delta)")
    })?;
    // Use the exact debt that will be recorded (floor((current_scaled + ceil(amount/index)) * index))
    // for the LTV check. Using current_underlying + amount would understate debt due to ceil
    // rounding and could allow a borrow that pushes LTV slightly above margin_rate.
    let debt_after_u128 = scaled_to_underlying_borrow(new_scaled, reserve.borrow_index)?;
    let debt_after = Uint128::from(debt_after_u128);

    let asset_prices = get_asset_prices_for_borrower(
        &deps.querier,
        &env.block.time,
        &contract,
        &borrower_collateral,
    )?;
    let (health, loan_to_value) = get_borrower_health(
        &contract,
        &contract.supported_collateral_assets,
        &asset_prices,
        &borrower_collateral,
        debt_after,
    )?;
    validate_borrower_is_healthy(health, loan_to_value, &contract)?;

    set_scaled_borrow(deps.storage, info.sender.as_str(), new_scaled)?;

    reserve.total_scaled_borrow = reserve
        .total_scaled_borrow
        .checked_add(scaled_delta)
        .ok_or_else(|| illegal_state("overflow: total_scaled_borrow + scaled_delta"))?;
    // Ensure the updated scaled total can still be materialized as underlying
    // (scaled * borrow_index) before persisting state.
    reserve.total_borrow()?;
    set_reserve_state_v1(deps.storage, &reserve)?;

    let send_msg = BankMsg::Send {
        to_address: info.sender.to_string(),
        amount: vec![Coin {
            denom: contract.lending_denom.name.clone(),
            amount,
        }],
    };

    Response::new()
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, info.sender.as_str())
        .add_attribute(ATTRIBUTE_AMOUNT, amount.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_delta.to_string())
        .attach_rates(&reserve, &contract.rate_params)
}
