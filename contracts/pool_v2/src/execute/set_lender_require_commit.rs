//! Contract owner only: set or clear per-address "require commit on exit" for lenders.

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::model::error::{illegal_argument, invalid_funds};
use crate::storage::{
    get_contract_state_v1, remove_lender_require_commit_on_exit,
    set_lender_require_commit_on_exit as store_lender_require_commit_on_exit,
};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::{assert_owner, ContractError};
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_lender_require_commit_on_exit";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may set lender commit-on-exit rules";

/// Set or clear per-address "require commit on exit" for a lender.
/// Some(true) = must pass commit_funds: true when withdrawing; Some(false) = no requirement; None = remove override.
pub fn set_lender_require_commit_on_exit(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    address: String,
    require: Option<bool>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let lender = deps.api.addr_validate(address.trim())?;

    match require {
        Some(true) => {
            ensure!(
                contract.commit_market_id.is_some(),
                illegal_argument(
                    "Commit market must be configured (commit_market_id) before requiring commit on exit",
                )
            );
            store_lender_require_commit_on_exit(deps.storage, &lender, true)?
        }
        Some(false) => store_lender_require_commit_on_exit(deps.storage, &lender, false)?,
        None => remove_lender_require_commit_on_exit(deps.storage, &lender),
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute("address", lender.as_str())
        .add_attribute(
            "require",
            require
                .map(|b| b.to_string())
                .unwrap_or_else(|| "default".to_string()),
        )
        .to_ok()
}
