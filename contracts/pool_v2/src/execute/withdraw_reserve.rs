//! Owner-only: withdraw the protocol's accrued reserve (reserve factor share of interest)
//! in lending denom to a specified recipient (or owner if omitted).

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_RECIPIENT};
use crate::model::error::{illegal_state, invalid_funds, ContractError};
use crate::storage::{get_contract_state_v1, set_reserve_state_v1};
use crate::utils::ownership::current_owner;
use crate::utils::update_reserve_indexes;
use crate::utils::WithRates;
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
    let amount = reserve.accrued_reserve;
    ensure!(amount > 0, illegal_state("No accrued reserve to withdraw"));

    reserve.accrued_reserve = 0;
    set_reserve_state_v1(deps.storage, &reserve)?;

    let send_msg = BankMsg::Send {
        to_address: to_address.to_string(),
        amount: vec![Coin {
            denom: contract.lending_denom.name.clone(),
            amount: Uint128::from(amount),
        }],
    };

    Response::new()
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_AMOUNT, amount.to_string())
        .add_attribute(ATTRIBUTE_RECIPIENT, to_address.as_str())
        .attach_rates(&reserve, &contract.rate_params)
}
