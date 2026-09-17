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

/// Withdraw the full accrued protocol reserve to the given recipient, or to the contract owner if recipient is None.
/// Owner only; no funds accepted. Updates reserve indexes first so accrued_reserve is current.
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

    // Protect lender principal even if accrued_reserve is ever over-booked: only the bank
    // balance above aggregate lender claims is withdrawable by the protocol.
    //
    // `lender_claims` is L − B, not the third value from `reserve_totals_and_cash_u128`
    // (`L − B − deficit`). That is safe only because of the deficit == 0 gate above; with a
    // positive deficit, cash would be the *smaller* bound and would overstate free surplus.
    // Do not "simplify" onto that third element: L − B is the more conservative payout cap
    // (larger claims → smaller amount).
    let (total_liquidity, total_borrow, _) = reserve_totals_and_cash_u128(&reserve)?;
    let lender_claims = total_liquidity.saturating_sub(total_borrow);
    let bank_balance = deps
        .querier
        .query_balance(
            env.contract.address.to_string(),
            contract.lending_denom.name.clone(),
        )?
        .amount
        .u128();
    let booked = reserve.accrued_reserve;
    let amount = booked.min(bank_balance.saturating_sub(lender_claims));
    ensure!(
        amount > 0,
        illegal_state("No solvent accrued reserve available to withdraw")
    );
    let writeoff = booked.saturating_sub(amount);

    // Close the entire booked bucket. Any amount rejected by the solvency cap was unbacked
    // and must not remain senior to future lender cash (or be spendable via EliminateDeficit).
    reserve.accrued_reserve = 0;
    set_reserve_state_v1(deps.storage, &reserve)?;

    let send_msg = BankMsg::Send {
        to_address: to_address.to_string(),
        amount: vec![Coin {
            denom: contract.lending_denom.name.clone(),
            amount: Uint128::from(amount),
        }],
    };

    let mut response = Response::new()
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_AMOUNT, amount.to_string())
        .add_attribute(ATTRIBUTE_RECIPIENT, to_address.as_str());
    if writeoff > 0 {
        response =
            response.add_attribute(ATTRIBUTE_UNBACKED_RESERVE_WRITEOFF, writeoff.to_string());
    }
    response.attach_rates(&reserve, &contract.rate_params)
}
