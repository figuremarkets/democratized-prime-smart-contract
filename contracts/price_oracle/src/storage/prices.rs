use crate::model::error::ContractError;
use crate::model::price::{PriceUpdateV1, PriceV1};
use cosmwasm_std::{Env, Order, Storage};
use cw_storage_plus::{Bound, Map};
use result_extensions::ResultExtensions;

const STORAGE_KEY_PRICES_V1: &str = "p1";
const PRICES_V1: Map<String, PriceV1> = Map::new(STORAGE_KEY_PRICES_V1);

/// Get the price of a single asset
pub fn try_get_usd_price_v1(
    store: &dyn Storage,
    asset: &str,
) -> Result<Option<PriceV1>, ContractError> {
    PRICES_V1
        .may_load(store, asset.to_owned())
        .map_err(ContractError::Std)
}

/// Save borrower by owner
pub fn save_usd_price_v1(
    store: &mut dyn Storage,
    asset: String,
    price: &PriceV1,
) -> Result<(), ContractError> {
    PRICES_V1
        .save(store, asset, price)
        .map_err(ContractError::Std)
}

/// Remove usd price by asset
pub fn remove_usd_price_v1(store: &mut dyn Storage, asset: String) {
    PRICES_V1.remove(store, asset)
}

/// Get page of prices
pub fn get_sorted_prices_v1(
    store: &dyn Storage,
    prev_asset: Option<String>,
    batch_size: u32,
) -> Result<Vec<(String, PriceV1)>, ContractError> {
    let start: Option<Bound<String>> = prev_asset.map(Bound::exclusive);

    PRICES_V1
        .range(store, start, None, Order::Ascending)
        .take(batch_size as usize)
        .map(|item| {
            let (asset, usd_price) = item.map_err(ContractError::Std)?;
            (asset, usd_price).to_ok()
        })
        .collect()
}

impl From<(&Env, &PriceUpdateV1)> for PriceV1 {
    fn from((env, price_update): (&Env, &PriceUpdateV1)) -> Self {
        Self {
            price_usd: price_update.usd,
            as_of_epoch_second: price_update.as_of.unwrap_or(env.block.time).seconds(),
        }
    }
}
