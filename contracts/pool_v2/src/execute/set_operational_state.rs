//! Contract owner only: set the pool's operational state (Active / Frozen / Paused).

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_STATE};
use crate::model::error::{invalid_funds, ContractError};
use crate::model::{ContractStateV1, OperationalState};
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use crate::utils::assert_owner_or_custodian;
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use result_extensions::ResultExtensions;

pub const ACTION: &str = "set_operational_state";
pub const ASSERT_PERMISSION_ERR: &str =
    "Only the contract owner or custodian may set operational state";

/// Set operational state. Contract owner only; no funds.
pub fn set_operational_state(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    state: OperationalState,
) -> Result<Response, ContractError> {
    assert_owner_or_custodian(deps.storage, &info.sender, ASSERT_PERMISSION_ERR)?;
    let mut contract: ContractStateV1 = get_contract_state_v1(deps.storage)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    contract.operational_state = state;
    set_contract_state_v1(deps.storage, &contract)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_STATE, format!("{:?}", state).to_lowercase())
        .to_ok()
}
