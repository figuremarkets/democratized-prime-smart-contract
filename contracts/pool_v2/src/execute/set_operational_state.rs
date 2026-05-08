//! Contract owner only: set the pool's operational state (Active / Frozen / Paused).

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_STATE};
use crate::model::error::{invalid_funds, ContractError};
use crate::model::OperationalState;
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_operational_state";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may set operational state";

/// Set operational state. Contract owner only; no funds.
pub fn set_operational_state(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    state: OperationalState,
) -> Result<Response, ContractError> {
    let mut contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    contract.operational_state = state;
    set_contract_state_v1(deps.storage, &contract)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_STATE, format!("{:?}", state).to_lowercase())
        .to_ok()
}
