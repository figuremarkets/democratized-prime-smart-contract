use crate::constants::DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS;
use crate::model::{
    error::{illegal_argument, ContractError},
    PriceV1,
};
use crate::utils::scale_price;
use cosmwasm_std::ensure;
use democratized_prime_lib::common::constants::MAX_DECIMAL_PRECISION;
use democratized_prime_lib::price_oracle::model::price::AssetPriceResponseV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Defines an asset mapping.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct AssetMappingV1 {
    /// Token denom of the display asset
    pub asset_id: String,

    /// Conversion of asset_id price to [`SaveAssetMappingRequestV1::alt_asset_id`].
    pub precision: u32,

    /// Defines the staleness threshold for the asset.
    /// Price data fetched for this asset are considered stale if at the time
    /// of use it is older than [`PriceV1::as_of_epoch_second`]
    #[serde(default = "AssetMappingV1::default_price_staleness_threshold")]
    pub staleness_threshold_seconds: u32,
}

impl AssetMappingV1 {
    pub fn default_price_staleness_threshold() -> u32 {
        DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS
    }

    /// Default is a 1:1 conversion
    pub fn default(id: String) -> Self {
        Self {
            asset_id: id,
            precision: 0_u32, // Maps to 10^0 -> 1
            staleness_threshold_seconds: Self::default_price_staleness_threshold(),
        }
    }
}

impl AssetMappingV1 {
    pub fn validate(&self) -> Result<(), ContractError> {
        // Check that the asset ID is non empty:
        ensure!(
            !self.asset_id.trim().is_empty(),
            illegal_argument("Asset ID cannot be empty")
        );

        // Check that the precision is less than [`Decimal256::DECIMAL_PLACES`]:
        ensure!(
            self.precision <= MAX_DECIMAL_PRECISION,
            illegal_argument(format!(
                "{}: precision must be <= {}",
                self.asset_id, MAX_DECIMAL_PRECISION
            ))
        );
        Ok(())
    }
}

pub trait IntoAssetPriceResponse {
    fn into_response(self) -> AssetPriceResponseV1;
}

impl IntoAssetPriceResponse for (AssetMappingV1, PriceV1) {
    fn into_response(self) -> AssetPriceResponseV1 {
        let (asset_mapping, price) = self;
        AssetPriceResponseV1 {
            price_usd: scale_price(price.price_usd, asset_mapping.precision).unwrap(),
            as_of_epoch_second: price.as_of_epoch_second,
            expiration_epoch_seconds: price.as_of_epoch_second
                + (asset_mapping.staleness_threshold_seconds as u64),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct SaveAssetMappingRequestV1 {
    // The base asset id. For example a provenance marker denom (e.g. nbtc.figure.se)
    pub alt_asset_id: String,

    // The display asset id and associated metadata (e.g. BTC)
    pub mapping: AssetMappingV1,
}

impl SaveAssetMappingRequestV1 {
    pub fn validate(&self) -> Result<(), ContractError> {
        // Check that the alternative asset ID is non empty:
        ensure!(
            !self.alt_asset_id.trim().is_empty(),
            illegal_argument(format!(
                "{}: alternative asset ID cannot be empty",
                self.mapping.asset_id
            ))
        );

        self.mapping.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod unit {
        use super::*;
        use crate::tests::constants::{ALT_ASSET_ID_BTC, ASSET_ID_BTC};
        use democratized_prime_lib::common::constants::MAX_DECIMAL_PRECISION;
        use result_extensions::ResultExtensions;

        #[test]
        fn accept_if_asset_mapping_precision_in_range() {
            let mapping = SaveAssetMappingRequestV1 {
                alt_asset_id: ALT_ASSET_ID_BTC.to_owned(),
                mapping: AssetMappingV1 {
                    asset_id: ASSET_ID_BTC.to_owned(),
                    precision: 9,
                    staleness_threshold_seconds: DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
                },
            };
            assert_eq!(mapping.validate(), Ok(()));
        }

        #[test]
        fn reject_if_asset_mapping_precision_is_out_of_range() {
            let mapping = SaveAssetMappingRequestV1 {
                alt_asset_id: ALT_ASSET_ID_BTC.to_owned(),
                mapping: AssetMappingV1 {
                    asset_id: ASSET_ID_BTC.to_owned(),
                    precision: MAX_DECIMAL_PRECISION + 1,
                    staleness_threshold_seconds: DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
                },
            };
            assert_eq!(
                mapping.validate(),
                illegal_argument(format!(
                    "{}: precision must be <= {}",
                    ASSET_ID_BTC, MAX_DECIMAL_PRECISION
                ))
                .to_err()
            );
        }

        #[test]
        fn reject_asset_id_is_empty() {
            let mapping = SaveAssetMappingRequestV1 {
                alt_asset_id: ALT_ASSET_ID_BTC.to_owned(),
                mapping: AssetMappingV1 {
                    asset_id: "".to_owned(),
                    precision: 9,
                    staleness_threshold_seconds: DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
                },
            };
            assert_eq!(
                mapping.validate(),
                illegal_argument("Asset ID cannot be empty").to_err()
            );
        }

        #[test]
        fn reject_alt_asset_id_is_empty() {
            let mapping = SaveAssetMappingRequestV1 {
                alt_asset_id: "".to_owned(),
                mapping: AssetMappingV1 {
                    asset_id: ASSET_ID_BTC.to_owned(),
                    precision: 9,
                    staleness_threshold_seconds: DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
                },
            };
            assert_eq!(
                mapping.validate(),
                illegal_argument(format!(
                    "{ASSET_ID_BTC}: alternative asset ID cannot be empty"
                ))
                .to_err()
            );
        }
    }
}
