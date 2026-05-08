use crate::model::error::ContractError;
use cosmwasm_std::Storage;
use cw_storage_plus::Item;
use democratized_prime_lib::price_oracle::model::ContractStateV1;

const STORAGE_KEY_CONTRACT_STATE: &str = "cs1";
pub const CONTRACT_STATE_V1: Item<ContractStateV1> = Item::new(STORAGE_KEY_CONTRACT_STATE);

/// Set/overwrites the state of the contract (singleton)
pub fn set_contract_state_v1(
    store: &mut dyn Storage,
    contract_state: &ContractStateV1,
) -> Result<(), ContractError> {
    CONTRACT_STATE_V1
        .save(store, contract_state)
        .map_err(ContractError::Std)
}
/// Read and return the state of the contract
pub fn get_contract_state_v1(store: &dyn Storage) -> Result<ContractStateV1, ContractError> {
    CONTRACT_STATE_V1.load(store).map_err(ContractError::Std)
}
