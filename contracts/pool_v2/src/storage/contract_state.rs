use crate::model::error::ContractError;
use crate::model::ContractStateV1;
use cosmwasm_std::Storage;
use cw_storage_plus::Item;

const KEY: &str = "cs1";
pub const ITEM: Item<ContractStateV1> = Item::new(KEY);

pub fn set_contract_state_v1(
    store: &mut dyn Storage,
    state: &ContractStateV1,
) -> Result<(), ContractError> {
    ITEM.save(store, state).map_err(ContractError::Std)
}

pub fn get_contract_state_v1(store: &dyn Storage) -> Result<ContractStateV1, ContractError> {
    ITEM.load(store).map_err(ContractError::Std)
}
