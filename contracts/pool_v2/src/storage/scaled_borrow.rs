use cosmwasm_std::{StdError, Storage};
use cw_storage_plus::Map;
use democratized_prime_lib::common::ContractError;
use result_extensions::ResultExtensions;

const KEY: &str = "sb1";
/// Map: borrower address -> scaled borrow (u128).
pub const SCALED_BORROW: Map<&[u8], u128> = Map::new(KEY);

pub fn get_scaled_borrow(store: &dyn Storage, owner: &str) -> Result<u128, ContractError> {
    SCALED_BORROW
        .may_load(store, owner.as_bytes())
        .map_err(ContractError::Std)?
        .unwrap_or(0)
        .to_ok()
}

pub fn set_scaled_borrow(
    store: &mut dyn Storage,
    owner: &str,
    amount: u128,
) -> Result<(), StdError> {
    if amount == 0 {
        SCALED_BORROW.remove(store, owner.as_bytes());
    } else {
        SCALED_BORROW.save(store, owner.as_bytes(), &amount)?;
    }
    Ok(())
}
