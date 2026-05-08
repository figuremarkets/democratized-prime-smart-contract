use crate::model::query::AssetRequirementV1;
use crate::model::{error::QueryError, StateResponseV1};
use crate::storage::{get_contract_state_v1, get_total_collateral_by_asset};
use crate::utils::compute_effective_reserve;
use cosmwasm_std::{to_json_binary, Binary, Deps, Env, Uint128};
use std::convert::TryInto;

/// Returns full state with effective reserve (indexes accrued to current block time), supported collateral, and total collateral held.
/// Reserve is returned as a response DTO including total_liquidity and total_borrow.
/// supported_collateral lists allowed assets and their haircuts; total_collateral_held is the pool's current exposure per asset.
pub fn query_state(deps: Deps, env: Env) -> Result<Binary, QueryError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let reserve = compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)
        .map_err(QueryError::Contract)?;
    let total_collateral_held: Vec<AssetRequirementV1> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| {
            let amount = get_total_collateral_by_asset(deps.storage, &a.asset_id).unwrap_or(0);
            AssetRequirementV1 {
                asset_id: a.asset_id.clone(),
                amount: Uint128::from(amount),
            }
        })
        .collect();
    let supported_collateral = contract.supported_collateral_assets.clone();
    to_json_binary(&StateResponseV1 {
        contract,
        reserve: reserve.try_into()?,
        supported_collateral,
        total_collateral_held,
    })
    .map_err(QueryError::Std)
}
