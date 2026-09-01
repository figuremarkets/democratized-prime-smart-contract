use crate::model::collateral::BorrowerCollateralV1;
use crate::model::contract_state::ContractStateV1;
use crate::model::error::{not_found, ContractError};
use cosmwasm_std::{Addr, QuerierWrapper, Timestamp};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use result_extensions::ResultExtensions;
use std::collections::HashSet;

/// Last-known prices for liquidation, plus denoms that must not be seized.
#[derive(Debug)]
pub struct LiquidationPrices {
    pub prices: PriceMapResponse,
    pub unpriceable: HashSet<String>,
}

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

/// Returns prices for the lending denom and collateral denoms that have a **fresh** stored price.
/// The lending denom must be present and fresh. Missing or stale collateral is omitted so callers
/// can value it at $0 ([`crate::utils::health::ZeroPricePolicy::TreatAsWorthless`]) instead of
/// freezing the borrower.
pub fn get_asset_prices_for_borrower(
    querier: &QuerierWrapper,
    block_time: &Timestamp,
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

    let lending_price = price_data
        .get(lending_denom)
        .ok_or_else(|| not_found(format!("Price of asset: {}", lending_denom)))?;
    if lending_price.is_stale(*block_time) {
        return Err(ContractError::StalePriceDataError {
            asset_id: lending_denom.clone(),
            expired_at: lending_price.expired_at(),
        });
    }

    let mut drop: Vec<String> = Vec::new();
    for asset_id in borrower_collateral.amounts.keys() {
        if asset_id == lending_denom {
            continue;
        }
        match price_data.get(asset_id) {
            None => {}
            Some(p) if p.is_stale(*block_time) || p.is_zero_price() => {
                drop.push(asset_id.clone());
            }
            Some(_) => {}
        }
    }
    for asset_id in drop {
        price_data.remove(&asset_id);
    }

    Ok(price_data)
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

/// True when the stored price is expired for at least `max_secs` (outer liquidation bound).
/// `max_secs == 0` treats any expired price as too old (last-known disabled).
fn liquidation_price_too_old(price: &AssetPriceResponseV1, now: Timestamp, max_secs: u64) -> bool {
    price.is_stale(now) && now.seconds().saturating_sub(price.expiration_epoch_seconds) >= max_secs
}

/// Prices for liquidation. A stored price is used even if stale, as long as it is still within
/// [`ContractStateV1::max_liquidation_staleness_seconds`] of expiration (last-known, Aave-style).
/// The lending denom must have a stored price within that bound. Collateral with no stored price,
/// or last-known older than the bound, is omitted and listed in [`LiquidationPrices::unpriceable`].
pub fn get_asset_prices_for_liquidation(
    querier: &QuerierWrapper,
    block_time: &Timestamp,
    contract_state: &ContractStateV1,
    borrower_collateral: &BorrowerCollateralV1,
) -> Result<LiquidationPrices, ContractError> {
    let lending_denom = &contract_state.lending_denom.name;
    let mut asset_ids: Vec<String> = vec![lending_denom.clone()];
    asset_ids.extend(borrower_collateral.amounts.keys().cloned());

    let mut price_data = get_price_from_oracle(
        querier,
        &contract_state.price_oracle_address,
        &asset_ids,
        true,
    )?;

    let lending_price = price_data.get(lending_denom).ok_or_else(|| {
        not_found(format!(
            "Price of lending denom is missing: {}",
            lending_denom
        ))
    })?;
    if liquidation_price_too_old(
        lending_price,
        *block_time,
        contract_state.max_liquidation_staleness_seconds,
    ) {
        return Err(ContractError::StalePriceDataError {
            asset_id: lending_denom.clone(),
            expired_at: lending_price.expired_at(),
        });
    }

    let mut unpriceable = HashSet::new();
    for asset_id in borrower_collateral.amounts.keys() {
        match price_data.get(asset_id) {
            None => {
                unpriceable.insert(asset_id.clone());
            }
            Some(p)
                if liquidation_price_too_old(
                    p,
                    *block_time,
                    contract_state.max_liquidation_staleness_seconds,
                ) =>
            {
                unpriceable.insert(asset_id.clone());
            }
            Some(_) => {}
        }
    }
    for asset_id in &unpriceable {
        price_data.remove(asset_id);
    }

    Ok(LiquidationPrices {
        prices: price_data,
        unpriceable,
    })
}
