use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON,
    ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON,
};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::model::CollateralAssetV1;
use crate::storage::{get_contract_state_v1, is_collateral_asset_in_use, set_contract_state_v1};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use result_extensions::ResultExtensions;
use std::collections::HashSet;

pub const ACTION: &str = "update_supported_collateral";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may update supported collateral";

/// Update supported collateral assets. Contract owner only; no funds. Cannot remove an asset that any
/// borrower currently holds.
pub fn update_supported_collateral(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    to_update: &[CollateralAssetV1],
    to_remove: &[String],
) -> Result<Response, ContractError> {
    let mut contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let mut all_asset_ids: Vec<String> = to_update.iter().map(|a| a.asset_id.clone()).collect();
    all_asset_ids.extend(to_remove.iter().cloned());
    let mut seen = HashSet::new();
    for id in &all_asset_ids {
        ensure!(
            !seen.contains(id),
            illegal_argument(format!("Duplicate asset id: {}", id))
        );
        seen.insert(id.clone());
    }

    for asset in to_update {
        asset.validate()?;
        ensure!(
            asset.asset_id != contract.lending_denom.name,
            illegal_argument(format!(
                "Collateral asset cannot be the lending denom ({})",
                asset.asset_id
            ))
        );
        if let Some(entry) = contract
            .supported_collateral_assets
            .iter_mut()
            .find(|e| e.asset_id == asset.asset_id)
        {
            *entry = asset.clone();
        } else {
            contract.supported_collateral_assets.push(asset.clone());
        }
    }

    let supported_ids: Vec<String> = contract
        .supported_collateral_assets
        .iter()
        .map(|e| e.asset_id.clone())
        .collect();
    let to_remove_filtered: Vec<String> = to_remove
        .iter()
        .filter(|id| supported_ids.contains(*id))
        .cloned()
        .collect();

    for asset_id in &to_remove_filtered {
        let in_use = is_collateral_asset_in_use(deps.storage, asset_id)?;
        ensure!(
            !in_use,
            illegal_state(format!(
                "Cannot remove collateral asset [{}]: held by at least one borrower",
                asset_id
            ))
        );
    }

    contract
        .supported_collateral_assets
        .retain(|e| !to_remove_filtered.contains(&e.asset_id));
    set_contract_state_v1(deps.storage, &contract)?;

    let updated_ids: Vec<String> = to_update.iter().map(|a| a.asset_id.clone()).collect();
    let removed_ids: Vec<String> = to_remove_filtered.clone();
    let mut res = Response::new().add_attribute(ATTRIBUTE_ACTION_NAME, ACTION);
    if !updated_ids.is_empty() {
        res = res.add_attribute(
            ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON,
            serde_json::to_string(&updated_ids).unwrap_or_default(),
        );
    }
    if !removed_ids.is_empty() {
        res = res.add_attribute(
            ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON,
            serde_json::to_string(&removed_ids).unwrap_or_default(),
        );
    }

    res.to_ok()
}
