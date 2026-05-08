use crate::model::{error::QueryError, ReserveResponseV1};
use crate::storage::get_contract_state_v1;
use crate::utils::{
    borrower_rate_from_utilization, compute_effective_reserve, lender_rate_from_utilization,
};
use cosmwasm_std::{to_json_binary, Binary, Deps, Env};
use std::convert::TryInto;

/// Returns effective reserve state as of current block time (indexes accrued to now)
/// plus current borrower and lender APRs so the UI can show live rates.
pub fn query_reserve(deps: Deps, env: Env) -> Result<Binary, QueryError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let reserve = compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)
        .map_err(QueryError::Contract)?;

    let utilization = reserve.utilization()?;
    let current_borrower_rate = borrower_rate_from_utilization(&contract.rate_params, utilization)
        .map_err(QueryError::Contract)?;
    let current_lender_rate =
        lender_rate_from_utilization(&contract.rate_params, utilization, current_borrower_rate)
            .map_err(QueryError::Contract)?;

    to_json_binary(&ReserveResponseV1 {
        reserve: reserve.try_into()?,
        current_borrower_rate: current_borrower_rate.to_string(),
        current_lender_rate: current_lender_rate.to_string(),
        utilization: utilization.to_string(),
    })
    .map_err(QueryError::Std)
}
