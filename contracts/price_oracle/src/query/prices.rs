use crate::model::{error::QueryError, AssetMappingV1, IntoAssetPriceResponse, PriceV1};
use crate::storage::{
    get_or_default_asset_mapping_v1, get_sorted_prices_v1, try_get_asset_mapping_v1,
    try_get_usd_price_v1,
};
use crate::utils::query_convert_to_binary;
use cosmwasm_std::{Binary, Storage};
use democratized_prime_lib::price_oracle::model::PriceMapResponse;
use std::collections::HashMap;

/// Query prices by asset ID.
///
/// If an asset provided in `assets` does not exist, an error will be returned.
///
/// # Arguments
///
/// * `current_time` - The time of the request.
/// * `assets` -  A [`Vec`] of asset IDs.
#[allow(clippy::single_match)]
pub fn query_prices_by_assets(
    store: &dyn Storage,
    assets: Vec<String>,
) -> Result<Binary, QueryError> {
    let mut prices: PriceMapResponse = HashMap::new();

    for requested_asset_id in assets {
        let (_alt_asset_id, display_asset_metadata): (String, AssetMappingV1) =
            get_or_default_asset_mapping_v1(store, &requested_asset_id)?;

        let price: PriceV1 = try_get_usd_price_v1(store, &display_asset_metadata.asset_id)?.ok_or(
            QueryError::NotFoundError {
                message: display_asset_metadata.asset_id.clone(),
            },
        )?;

        prices.insert(
            requested_asset_id,
            (display_asset_metadata, price).into_response(),
        );
    }

    query_convert_to_binary(&prices)
}

/// Batch query assets.
///
/// # Arguments
///
/// * `current_time` - The time of the request.
/// * `prev_asset` - The starting asset ID to retrieve prices from.
/// * `batch_size` - The maximum number of assets to be returned.
pub fn query_prices_batch(
    store: &dyn Storage,
    prev_asset: Option<String>,
    batch_size: u32,
) -> Result<Binary, QueryError> {
    let prices = get_sorted_prices_v1(store, prev_asset, batch_size)?;

    let mut price_map: PriceMapResponse = HashMap::new();

    for (asset_id, price) in prices {
        let (display_asset_id, asset_metadata): (String, AssetMappingV1) =
            try_get_asset_mapping_v1(store, &asset_id)?.map_or(
                (asset_id.clone(), AssetMappingV1::default(asset_id.clone())),
                |am| (asset_id, am),
            );

        price_map.insert(display_asset_id, (asset_metadata, price).into_response());
    }

    query_convert_to_binary(&price_map)
}
