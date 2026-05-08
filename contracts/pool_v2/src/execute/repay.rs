use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_BORROWER, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, ContractError};
use crate::storage::{
    get_contract_state_v1, get_scaled_borrow, set_reserve_state_v1, set_scaled_borrow,
};
use crate::utils::{
    scaled_to_underlying_borrow, underlying_to_scaled_borrow, update_reserve_indexes,
    validate_single_coin_denom, WithRates,
};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};

pub const ACTION: &str = "repay";

/// Repay borrow using funds sent in the message. We apply min(sent amount, current debt) so users
/// can send "pay off full" amounts without failing when interest accrues between query and execute.
/// Any excess over current debt is sent back to the sender.
pub fn repay(deps: DepsMut, env: Env, info: MessageInfo) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let amount = validate_single_coin_denom(&info, &contract.lending_denom, Uint128::new(1))?;
    let amount = amount.u128();

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;

    let scaled_debt = get_scaled_borrow(deps.storage, info.sender.as_str())?;
    ensure!(scaled_debt > 0, illegal_argument("No borrow to repay"));
    let debt_underlying = scaled_to_underlying_borrow(scaled_debt, reserve.borrow_index)?;
    let repay_underlying = amount.min(debt_underlying);
    // When repaying in full (repay_underlying >= debt_underlying), use scaled_debt directly to
    // avoid double-floor dust: floor(debt_underlying/index) can be < scaled_debt, leaving
    // irremovable scaled debt (e.g. scaled_debt=99, index=1.05 → debt_underlying=103,
    // floor(103/1.05)=98, leaving 1 scaled unit stuck forever).
    let scaled_repay = if repay_underlying >= debt_underlying {
        scaled_debt
    } else {
        underlying_to_scaled_borrow(repay_underlying, reserve.borrow_index)?
    };

    let new_scaled = scaled_debt.checked_sub(scaled_repay).ok_or_else(|| {
        illegal_state("underflow: borrower scaled debt (scaled_debt - scaled_repay)")
    })?;
    set_scaled_borrow(deps.storage, info.sender.as_str(), new_scaled)?;

    reserve.total_scaled_borrow = reserve
        .total_scaled_borrow
        .checked_sub(scaled_repay)
        .ok_or_else(|| illegal_state("underflow: total_scaled_borrow - scaled_repay"))?;
    set_reserve_state_v1(deps.storage, &reserve)?;

    let mut res = Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, info.sender.as_str())
        .add_attribute(ATTRIBUTE_AMOUNT, repay_underlying.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_repay.to_string());

    // Refund excess: user may send more than current debt (e.g. "pay full"); we only apply min.
    if amount > repay_underlying {
        let excess = Uint128::new(amount)
            .checked_sub(Uint128::new(repay_underlying))?
            .u128();
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
