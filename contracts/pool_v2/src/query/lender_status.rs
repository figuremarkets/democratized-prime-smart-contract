//! GetLenderStatus query: per-address "require commit on exit" for lenders.

use crate::model::query::LenderStatusResponseV1;
use crate::storage::get_lender_require_commit_on_exit;
use cosmwasm_std::{to_json_binary, Binary, Deps};
use democratized_prime_lib::common::QueryError;

/// Returns whether the given address has "require commit on exit" set (must pass commit_funds: true to withdraw).
/// Supply balance is obtained from the repo_token_cw20 contract (Balance / TokenInfo), not from this query.
pub fn query_lender_status(deps: Deps, address: &str) -> Result<Binary, QueryError> {
    let addr = deps.api.addr_validate(address.trim())?;
    let require_commit_on_exit =
        get_lender_require_commit_on_exit(deps.storage, &addr).map_err(QueryError::Contract)?;
    to_json_binary(&LenderStatusResponseV1 {
        require_commit_on_exit,
    })
    .map_err(QueryError::Std)
}
