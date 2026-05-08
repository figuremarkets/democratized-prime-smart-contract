use crate::model::error::ContractError;
use crate::model::ReserveStateV1;
use cosmwasm_std::Storage;
use cw_storage_plus::Item;

const KEY: &str = "res1";
const ITEM: Item<ReserveStateV1> = Item::new(KEY);

pub fn set_reserve_state_v1(
    store: &mut dyn Storage,
    state: &ReserveStateV1,
) -> Result<(), ContractError> {
    ITEM.save(store, state).map_err(ContractError::Std)
}

pub fn get_reserve_state_v1(store: &dyn Storage) -> Result<ReserveStateV1, ContractError> {
    ITEM.load(store).map_err(ContractError::Std)
}
