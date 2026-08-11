use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_PREVIOUS_PRICES_JSON, ATTRIBUTE_UPDATED_PRICES_JSON,
};
use crate::model::{error::ContractError, IntoAssetPriceResponse, PriceUpdateV1, PriceV1};
use crate::storage::{get_or_default_asset_mapping_v1, save_usd_price_v1, try_get_usd_price_v1};
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

    let mut previous_prices: BTreeMap<String, AssetPriceResponseV1> = BTreeMap::new();
    let mut updated_prices: BTreeMap<String, AssetPriceResponseV1> = BTreeMap::new();

    for price_update in price_updates {
        let (display_asset_id, asset_metadata) =
            get_or_default_asset_mapping_v1(deps.storage, &price_update.asset)?;

        if let Some(previous_price) = try_get_usd_price_v1(deps.storage, &price_update.asset)? {
            previous_prices.insert(
                display_asset_id.clone(),
                (asset_metadata.clone(), previous_price).into_response()?,
            );
        }

        price_update.validate(env.block.time)?;
        let new_price: PriceV1 = (&env, &price_update).into();
        save_usd_price_v1(deps.storage, price_update.asset.clone(), &new_price)?;
        updated_prices.insert(
            display_asset_id,
            (asset_metadata, new_price).into_response()?,
        );
    }

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ACTION_VALUE)
        .add_attribute(
            ATTRIBUTE_PREVIOUS_PRICES_JSON,
            serde_json::to_string(&previous_prices).unwrap_or_default(),
        )
        .add_attribute(
            ATTRIBUTE_UPDATED_PRICES_JSON,
            serde_json::to_string(&updated_prices).unwrap_or_default(),
        )
        .to_ok()
}
