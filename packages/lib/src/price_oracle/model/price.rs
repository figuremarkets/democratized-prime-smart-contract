use cosmwasm_std::{Decimal256, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct AssetPriceResponseV1 {
    /// USD price of the requested asset id
    /// Querying a base asset (e.g. nbtc.figure.se) will return the price of the base asset in USD
    /// Querying a display asset name (e.g. BTC) will return the price of the display asset in USD
    pub price_usd: Decimal256,

    /// Epoch second timestamp of update
    pub as_of_epoch_second: u64,

    /// The expiration time of the price in epoch seconds. This is defined as the
    /// [`AssetPriceResponseV1::as_of_epoch_second`] + the staleness threshold for the asset.
    pub expiration_epoch_seconds: u64,
}

impl AssetPriceResponseV1 {
    /// Tests if the price is considered stale by comparing the current time
    /// against [`AssetPriceResponseV1::expiration_epoch_seconds`].
    pub fn is_stale(&self, at: Timestamp) -> bool {
        at.seconds() >= self.expiration_epoch_seconds
    }

    pub fn expired_at(&self) -> Timestamp {
        Timestamp::from_seconds(self.expiration_epoch_seconds)
    }
}

/// Map of denom to PriceV1 record
/// K: denom
/// V: price & metadata of asset
pub type PriceMapResponse = HashMap<String, AssetPriceResponseV1>;
