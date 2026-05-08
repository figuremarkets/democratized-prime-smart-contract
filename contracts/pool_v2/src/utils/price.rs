use crate::model::collateral::BorrowerCollateralV1;
use crate::model::contract_state::ContractStateV1;
use crate::model::error::ContractError;
use cosmwasm_std::{Addr, QuerierWrapper, Timestamp};
use democratized_prime_lib::price_oracle::model::PriceMapResponse;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg as PriceOracleQueryMsg;
use result_extensions::ResultExtensions;

/// Fetches asset prices from the price oracle contract.
pub fn get_price_from_oracle(
    querier: &QuerierWrapper,
    oracle_addr: &Addr,
    assets: &[String],
) -> Result<PriceMapResponse, ContractError> {
    let request_body = PriceOracleQueryMsg::GetPricesByAsset {
        assets: assets.to_owned(),
    };
    querier
        .query_wasm_smart::<PriceMapResponse>(oracle_addr, &request_body)?
        .to_ok()
}

/// Returns prices for lending denom and all collateral denoms (for health/LTV).
pub fn get_asset_prices_for_borrower(
    querier: &QuerierWrapper,
    block_time: &Timestamp,
    contract_state: &ContractStateV1,
    borrower_collateral: &BorrowerCollateralV1,
) -> Result<PriceMapResponse, ContractError> {
    let mut asset_ids: Vec<String> = vec![contract_state.lending_denom.name.clone()];
    asset_ids.extend(borrower_collateral.amounts.keys().cloned());

    // Fetch price data from the oracle:
    let price_data =
        get_price_from_oracle(querier, &contract_state.price_oracle_address, &asset_ids)?;

    // Validate that each price is not stale:
    for (asset_id, price) in price_data.iter() {
        if price.is_stale(*block_time) {
            return Err(ContractError::StalePriceDataError {
                asset_id: asset_id.clone(),
                expired_at: price.expired_at(),
            });
        }
    }

    Ok(price_data)
}
