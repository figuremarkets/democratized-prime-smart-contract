use crate::constants::{ATTRIBUTE_ACTION_NAME, CONTRACT_NAME, CONTRACT_VERSION};
use crate::model::error::ContractError;
use crate::msg::instantiate::InstantiateMsg;
use crate::storage::contract_state::set_contract_state_v1;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use cw2::set_contract_version;
use cw_ownable::initialize_owner;
use democratized_prime_lib::price_oracle::model::ContractStateV1;
use result_extensions::ResultExtensions;

// Instantiate smart contract and store initial state
pub fn instantiate_contract(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_state_v1(deps.storage, &ContractStateV1 {})?;
    let owner = deps.api.addr_validate(msg.owner.as_str())?;
    initialize_owner(deps.storage, deps.api, Some(owner.as_str()))?;

    // Update contract version
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    // Create response message
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, "instantiate")
        .to_ok()
}
