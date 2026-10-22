//! Contract owner only: set or clear per-address "require commit on exit" for borrowers.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ADDRESS, ATTRIBUTE_REQUIRE};
use crate::model::error::{illegal_argument, invalid_funds};
use crate::storage::{
    get_contract_state_v1, remove_borrower_require_commit,
    set_borrower_require_commit as store_borrower_require_commit_on_exit,
};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::{assert_owner, ContractError};
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_borrower_require_commit";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may set borrower commit rules";

/// Set or clear per-address "require commit on exit" for a borrower.
/// When true, fund committment will occur
/// - when borrowing
/// - when there is an excess of funds when repaying a debt
/// - removing collateral
pub fn set_borrower_require_commit_on_exit(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    address: String,
    require: Option<bool>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let borrower = deps.api.addr_validate(address.trim())?;

    match require {
        Some(true) => {
            ensure!(
                contract.commit_market_id.is_some(),
                illegal_argument(
                    "Commit market must be configured (commit_market_id) before requiring commit",
                )
            );
            store_borrower_require_commit_on_exit(deps.storage, &borrower, true)?
        }
        Some(false) => store_borrower_require_commit_on_exit(deps.storage, &borrower, false)?,
        None => remove_borrower_require_commit(deps.storage, &borrower),
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_ADDRESS, borrower.as_str())
        .add_attribute(
            ATTRIBUTE_REQUIRE,
            require
                .map(|b| b.to_string())
                .unwrap_or_else(|| "default".to_string()),
        )
        .to_ok()
}
