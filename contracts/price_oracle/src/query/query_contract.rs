use crate::model::error::QueryError;
use crate::storage::contract_state::get_contract_state_v1;
use crate::utils::misc_utils::query_convert_to_binary;
use cosmwasm_std::{Binary, Storage};
use democratized_prime_lib::price_oracle::model::ContractStateV1;

/// Get all states
pub fn query_state(store: &dyn Storage) -> Result<Binary, QueryError> {
    let state: ContractStateV1 = get_contract_state_v1(store).map_err(QueryError::Contract)?;
    query_convert_to_binary(&state)
}
