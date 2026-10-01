//! Owner-only: withdraw the protocol's accrued reserve (reserve factor share of interest)
//! in lending denom to a specified recipient (or owner if omitted).

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_RECIPIENT,
    ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF,
};
use crate::model::error::{illegal_state, invalid_funds, ContractError};
use crate::storage::{get_contract_state_v1, set_reserve_state_v1};
use crate::utils::ownership::current_owner;
use crate::utils::{reserve_totals_and_cash_u128, update_reserve_indexes, WithRates};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};
use democratized_prime_lib::common::assert_owner;

pub const ACTION: &str = "withdraw_reserve";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may withdraw accrued reserve";

/// Withdraw backed accrued protocol reserve to the given recipient, or to the contract owner if recipient is None.
/// Owner only; no funds accepted. Updates reserve indexes first so accrued_reserve is current.
/// Uncollected reserve stays booked. Only reserve above `bank + B − L` is written off.
pub fn withdraw_reserve(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let to_address = match &recipient {
        Some(addr) => deps.api.addr_validate(addr)?,
        None => current_owner(deps.storage)?,
    };

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    ensure!(
        reserve.deficit_underlying == 0,
        illegal_state(
            "Cannot withdraw reserve while deficit_underlying > 0; use EliminateDeficit first"
        )
    );
    ensure!(
        reserve.accrued_reserve > 0,
        illegal_state("No accrued reserve to withdraw")
    );

    // Fees are senior to lender principal. `free = (bank + B) − L` is the signed surplus over
    // lender claims: when borrowers owe more than lenders (B > L), coins still in the bank back
    // fees that repayment will restore. Uncollected reserve stays booked (`backed − pay`).
    // Only reserve above `bank + B − L` is written off.
    //
    // Do not use the third value from `reserve_totals_and_cash_u128`. That cash figure saturates
    // `L − B` at zero (and also subtracts deficit), so it drops the sign and understates free
    // surplus whenever B > L. The deficit == 0 gate above is what makes `bank + B − L` the
    // pool's free surplus; it is not a license to substitute the saturated cash figure.
    let (total_liquidity, total_borrow, _) = reserve_totals_and_cash_u128(&reserve)?;
    let bank_balance = deps
        .querier
        .query_balance(
            env.contract.address.to_string(),
            contract.lending_denom.name.clone(),
        )?
        .amount
        .u128();
    let booked = reserve.accrued_reserve;
    let free = bank_balance
        .checked_add(total_borrow)
        .ok_or_else(|| illegal_state("bank + total_borrow overflow"))?
        .saturating_sub(total_liquidity);
    let backed = booked.min(free);
    let pay = backed.min(bank_balance);
    ensure!(
        pay > 0,
        illegal_state("No solvent accrued reserve available to withdraw")
    );
    let writeoff = booked
        .checked_sub(backed)
        .ok_or_else(|| illegal_state("unbacked reserve writeoff underflow"))?;
    // Backed coins the bank cannot send yet stay booked for a later withdraw.
    reserve.accrued_reserve = backed
        .checked_sub(pay)
        .ok_or_else(|| illegal_state("accrued_reserve remainder underflow"))?;
    set_reserve_state_v1(deps.storage, &reserve)?;

    let send_msg = BankMsg::Send {
        to_address: to_address.to_string(),
        amount: vec![Coin {
            denom: contract.lending_denom.name.clone(),
            amount: Uint128::from(pay),
        }],
    };

    let mut response = Response::new()
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_AMOUNT, pay.to_string())
        .add_attribute(ATTRIBUTE_RECIPIENT, to_address.as_str());
    if writeoff > 0 {
        response =
            response.add_attribute(ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF, writeoff.to_string());
    }
    response.attach_rates(&reserve, &contract.rate_params)
}
