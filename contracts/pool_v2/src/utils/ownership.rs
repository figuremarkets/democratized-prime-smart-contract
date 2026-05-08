//! Helpers around [`cw_ownable`] for this contract.

use crate::model::error::{illegal_state, ContractError};
use cosmwasm_std::Addr;
use cosmwasm_std::Storage;
use cw_ownable::get_ownership;

/// Current owner address, or error if ownership was renounced.
pub fn current_owner(storage: &dyn Storage) -> Result<Addr, ContractError> {
    get_ownership(storage)
        .map_err(ContractError::Std)?
        .owner
        .ok_or_else(|| illegal_state("Contract ownership has been renounced"))
}
