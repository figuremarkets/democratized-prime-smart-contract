use crate::model::collateral::BorrowerCollateralV1;
use crate::model::contract_state::ContractStateV1;
use crate::model::error::{not_found, ContractError};
use cosmwasm_std::{Addr, Decimal256, QuerierWrapper, Timestamp};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use result_extensions::ResultExtensions;

/// Fetches asset prices from the price oracle contract.
pub fn get_price_from_oracle(
    querier: &QuerierWrapper,
    oracle_addr: &Addr,
    assets: &[String],
    skip_missing: bool,
) -> Result<PriceMapResponse, ContractError> {
    let request_body = PriceOracleQueryMsg::GetPricesByAsset {
        assets: assets.to_owned(),
        skip_missing,
    };
    querier
        .query_wasm_smart::<PriceMapResponse>(oracle_addr, &request_body)?
        .to_ok()
}

/// Returns prices for lending denom and all collateral denoms (for health/LTV).
/// Errors if any requested price is missing or stale.
pub fn get_asset_prices_for_borrower(
    querier: &QuerierWrapper,
    block_time: &Timestamp,
    contract_state: &ContractStateV1,
    borrower_collateral: &BorrowerCollateralV1,
) -> Result<PriceMapResponse, ContractError> {
    let mut asset_ids: Vec<String> = vec![contract_state.lending_denom.name.clone()];
    asset_ids.extend(borrower_collateral.amounts.keys().cloned());
    require_fresh_asset_prices(
        querier,
        block_time,
        &contract_state.price_oracle_address,
        &asset_ids,
    )
}

/// Ensures each listed asset has a stored oracle price that is not stale.
pub fn require_fresh_asset_prices(
    querier: &QuerierWrapper,
    block_time: &Timestamp,
    oracle_addr: &Addr,
    assets: &[String],
) -> Result<PriceMapResponse, ContractError> {
    if assets.is_empty() {
        return Ok(PriceMapResponse::new());
    }
    let price_data = get_price_from_oracle(querier, oracle_addr, assets, false)?;
    for asset_id in assets {
        let price = price_data
            .get(asset_id)
            .ok_or_else(|| not_found(format!("Price of asset: {}", asset_id)))?;
        if price.is_stale(*block_time) {
            return Err(ContractError::StalePriceDataError {
                asset_id: asset_id.clone(),
                expired_at: price.expired_at(),
            });
        }
    }
    Ok(price_data)
}

/// Zero USD placeholder so LTV / min-repay treat collateral with **no stored price** as worthless.
/// `expiration_epoch_seconds` is far enough in the future that `is_stale` is false, but small
/// enough that `Timestamp::from_seconds` does not overflow (nanos = secs * 1e9).
fn zero_collateral_price() -> AssetPriceResponseV1 {
    AssetPriceResponseV1::new(Decimal256::zero(), 0, u32::MAX as u64)
}

/// Prices for liquidation. A stored price is used even if stale (last-known, Aave-style) so
/// liquidations are not frozen by a paused feed. The lending denom must still have a stored
/// price. Collateral with **no** stored price is included at $0; callers must not seize it.
pub fn get_asset_prices_for_liquidation(
    querier: &QuerierWrapper,
    contract_state: &ContractStateV1,
    borrower_collateral: &BorrowerCollateralV1,
) -> Result<PriceMapResponse, ContractError> {
    let lending_denom = &contract_state.lending_denom.name;
    let mut asset_ids: Vec<String> = vec![lending_denom.clone()];
    asset_ids.extend(borrower_collateral.amounts.keys().cloned());

    let mut price_data = get_price_from_oracle(
        querier,
        &contract_state.price_oracle_address,
        &asset_ids,
        true,
    )?;

    if !price_data.contains_key(lending_denom) {
        return Err(not_found(format!(
            "Price of lending denom is missing: {}",
            lending_denom
        )));
    }

    for asset_id in borrower_collateral.amounts.keys() {
        if !price_data.contains_key(asset_id) {
            price_data.insert(asset_id.clone(), zero_collateral_price());
        }
    }

    Ok(price_data)
}
