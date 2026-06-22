use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_PRICE_MAP_JSON};
use crate::model::{error::ContractError, AssetMappingV1, IntoAssetPriceResponse, PriceUpdateV1};
use crate::storage::{get_sorted_prices_v1, save_usd_price_v1, try_get_asset_mapping_v1};
use crate::utils::validate_name_uniqueness;
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;
use result_extensions::ResultExtensions;
use std::collections::BTreeMap;

pub const ATTRIBUTE_ACTION_VALUE: &str = "set_asset_prices";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may update asset prices";

/// Attempt to update asset prices.
///
/// # Arguments
///
/// * `price_updates` - Updates asset prices.
pub fn try_update_asset_prices(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    price_updates: Vec<PriceUpdateV1>,
) -> Result<Response, ContractError> {
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;

    // Validate unique asset IDs:
    let asset_ids = price_updates
        .iter()
        .map(|update| update.asset.clone())
        .collect::<Vec<_>>();

    validate_name_uniqueness(&asset_ids)?;

    for price_update in price_updates {
        price_update.validate(env.block.time)?;

        save_usd_price_v1(
            deps.storage,
            price_update.asset.clone(),
            &(&env, &price_update).into(),
        )?;
    }

    let prices = get_sorted_prices_v1(deps.storage, None, u32::MAX)?;
    let mut price_map: BTreeMap<String, AssetPriceResponseV1> = BTreeMap::new();
    for (asset_id, price) in prices {
        let (display_asset_id, asset_metadata): (String, AssetMappingV1) =
            try_get_asset_mapping_v1(deps.storage, &asset_id)?.map_or(
                (asset_id.clone(), AssetMappingV1::default(asset_id.clone())),
                |am| (asset_id, am),
            );
        price_map.insert(display_asset_id, (asset_metadata, price).into_response());
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ACTION_VALUE)
        .add_attribute(
            ATTRIBUTE_PRICE_MAP_JSON,
            serde_json::to_string(&price_map).unwrap_or_default(),
        )
        .to_ok()
}
