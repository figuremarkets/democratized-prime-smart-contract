//! GetBorrowerStatus query: per-address require-commit for borrowers.

use crate::model::query::BorrowerStatusResponseV1;
use crate::storage::get_borrower_require_commit;
use cosmwasm_std::{to_json_binary, Binary, Deps};
use democratized_prime_lib::common::QueryError;

/// Returns whether the given address is marked require-commit.
pub fn query_borrower_status(deps: Deps, address: &str) -> Result<Binary, QueryError> {
    let addr = deps.api.addr_validate(address.trim())?;
    let require_commit =
        get_borrower_require_commit(deps.storage, &addr).map_err(QueryError::Contract)?;
    to_json_binary(&BorrowerStatusResponseV1 { require_commit }).map_err(QueryError::Std)
}
