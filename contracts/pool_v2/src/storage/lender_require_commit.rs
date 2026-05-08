//! Per-address "require commit (e.g. to exchange) on lender withdraw".
//! We only store when the requirement is true; no entry means no requirement (false).

use cosmwasm_std::{Addr, Storage};
use cw_storage_plus::Map;

use crate::model::error::ContractError;

const STORAGE_KEY_LENDER_REQUIRE_COMMIT: &str = "lrc1";
const LENDER_REQUIRE_COMMIT_ON_EXIT: Map<&str, ()> = Map::new(STORAGE_KEY_LENDER_REQUIRE_COMMIT);

/// Returns true if this lender must pass commit_funds when withdrawing; false if not set.
pub fn get_lender_require_commit_on_exit(
    store: &dyn Storage,
    lender: &Addr,
) -> Result<bool, ContractError> {
    LENDER_REQUIRE_COMMIT_ON_EXIT
        .may_load(store, lender.as_str())
        .map_err(ContractError::Std)
        .map(|v| v.is_some())
}

/// Set the per-address "require commit on exit". true = require; false = clear (no requirement).
pub fn set_lender_require_commit_on_exit(
    store: &mut dyn Storage,
    lender: &Addr,
    require: bool,
) -> Result<(), ContractError> {
    if require {
        LENDER_REQUIRE_COMMIT_ON_EXIT
            .save(store, lender.as_str(), &())
            .map_err(ContractError::Std)
    } else {
        remove_lender_require_commit_on_exit(store, lender);
        Ok(())
    }
}

/// Clear the per-address requirement (same as set to false).
pub fn remove_lender_require_commit_on_exit(store: &mut dyn Storage, lender: &Addr) {
    LENDER_REQUIRE_COMMIT_ON_EXIT.remove(store, lender.as_str());
}
