use crate::model::error::{illegal_state, ContractError};
use crate::model::BorrowerCollateralV1;
use cosmwasm_std::Storage;
use cw_storage_plus::Map;

const BORROWER_KEY: &str = "bc1";
const TOTAL_BY_ASSET_KEY: &str = "tca1";

/// Per-borrower collateral amounts. Key: borrower address (String).
pub const BORROWER_COLLATERAL: Map<String, BorrowerCollateralV1> = Map::new(BORROWER_KEY);
/// Total amount of each asset held as collateral (sum across all borrowers). Key: asset_id (String). O(1) for "in use" check.
const TOTAL_COLLATERAL_BY_ASSET: Map<String, u128> = Map::new(TOTAL_BY_ASSET_KEY);

pub fn get_borrower_collateral(
    store: &dyn Storage,
    borrower: &str,
) -> Result<BorrowerCollateralV1, ContractError> {
    Ok(BORROWER_COLLATERAL
        .may_load(store, borrower.to_string())
        .map_err(ContractError::Std)?
        .unwrap_or_default())
}

pub fn set_borrower_collateral(
    store: &mut dyn Storage,
    borrower: &str,
    data: &BorrowerCollateralV1,
) -> Result<(), ContractError> {
    let key = borrower.to_string();
    if data.amounts.is_empty() {
        BORROWER_COLLATERAL.remove(store, key);
    } else {
        BORROWER_COLLATERAL.save(store, key, data)?;
    }
    Ok(())
}

/// Total collateral amount for an asset (sum across all borrowers). Used for O(1) "in use" check.
pub fn get_total_collateral_by_asset(
    store: &dyn Storage,
    asset_id: &str,
) -> Result<u128, ContractError> {
    Ok(TOTAL_COLLATERAL_BY_ASSET
        .may_load(store, asset_id.to_string())
        .map_err(ContractError::Std)?
        .unwrap_or(0))
}

/// Returns true if any borrower has a non-zero amount of the given collateral asset. O(1).
pub fn is_collateral_asset_in_use(
    store: &dyn Storage,
    asset_id: &str,
) -> Result<bool, ContractError> {
    Ok(get_total_collateral_by_asset(store, asset_id)? > 0)
}

/// Call when collateral is added: increases total for each asset. Must be kept in sync with borrower balances.
pub fn add_total_collateral(
    store: &mut dyn Storage,
    asset_id: &str,
    amount: u128,
) -> Result<(), ContractError> {
    let cur = get_total_collateral_by_asset(store, asset_id)?;
    let new_total = cur
        .checked_add(amount)
        .ok_or_else(|| illegal_state("overflow: total_collateral_by_asset (cur + amount)"))?;
    let key = asset_id.to_string();
    if new_total == 0 {
        TOTAL_COLLATERAL_BY_ASSET.remove(store, key);
    } else {
        TOTAL_COLLATERAL_BY_ASSET.save(store, key, &new_total)?;
    }
    Ok(())
}

/// Call when collateral is removed: decreases total for each asset. Errors if would go below zero.
pub fn subtract_total_collateral(
    store: &mut dyn Storage,
    asset_id: &str,
    amount: u128,
) -> Result<(), ContractError> {
    let cur = get_total_collateral_by_asset(store, asset_id)?;
    let new_total = cur
        .checked_sub(amount)
        .ok_or_else(|| illegal_state("total collateral by asset would underflow"))?;
    let key = asset_id.to_string();
    if new_total == 0 {
        TOTAL_COLLATERAL_BY_ASSET.remove(store, key);
    } else {
        TOTAL_COLLATERAL_BY_ASSET.save(store, key, &new_total)?;
    }
    Ok(())
}
