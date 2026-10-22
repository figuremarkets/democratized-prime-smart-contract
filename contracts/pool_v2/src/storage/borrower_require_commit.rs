//! Per-address "require commit (e.g. to exchange) on borrow".
//! We only store when the requirement is true; no entry means no requirement (false).

use cosmwasm_std::{Addr, Storage};
use cw_storage_plus::Map;

use crate::model::error::ContractError;

const STORAGE_KEY_BORROWER_REQUIRE_COMMIT: &str = "brc1";
const BORROWER_REQUIRE_COMMIT_ON_EXIT: Map<&str, ()> =
    Map::new(STORAGE_KEY_BORROWER_REQUIRE_COMMIT);

/// Returns true if this borrower must pass commit_funds when withdrawing; false if not set.
pub fn get_borrower_require_commit(
    store: &dyn Storage,
    borrower: &Addr,
) -> Result<bool, ContractError> {
    BORROWER_REQUIRE_COMMIT_ON_EXIT
        .may_load(store, borrower.as_str())
        .map_err(ContractError::Std)
        .map(|v| v.is_some())
}

/// Set the per-address "require commit". true = require; false = clear (no requirement).
pub fn set_borrower_require_commit(
    store: &mut dyn Storage,
    lender: &Addr,
    require: bool,
) -> Result<(), ContractError> {
    if require {
        BORROWER_REQUIRE_COMMIT_ON_EXIT
            .save(store, lender.as_str(), &())
            .map_err(ContractError::Std)
    } else {
        remove_borrower_require_commit(store, lender);
        Ok(())
    }
}

/// Clear the per-address requirement (same as set to false).
pub fn remove_borrower_require_commit(store: &mut dyn Storage, lender: &Addr) {
    BORROWER_REQUIRE_COMMIT_ON_EXIT.remove(store, lender.as_str());
}
