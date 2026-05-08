use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct PriceUpdateV1 {
    /// Asset identifier (e.g. marker denom)
    pub asset: String,

    /// USD price of the asset
    pub usd: Decimal256,

    /// Epoch second timestamp of update
    pub as_of: Option<Timestamp>,
}

impl PriceUpdateV1 {
    pub fn validate(&self, current_time: Timestamp) -> Result<(), ContractError> {
        // Check that the asset name is not empty:
        ensure!(
            !self.asset.trim().is_empty(),
            illegal_argument("Asset name cannot be empty")
        );

        // Check that the USD price of the asset is non-zero:
        ensure!(
            !self.usd.is_zero(),
            illegal_argument(format!("{}: asset price must be > 0", self.asset))
        );

        // If a timestamp is provided, check that it's not too far in the
        // past or future:
        if let Some(as_of) = self.as_of {
            ensure!(
                as_of >= current_time.minus_days(30),
                illegal_argument("Price timestamp too old")
            );
            ensure!(
                as_of <= current_time.plus_minutes(5),
                illegal_argument("Price timestamp too far into the future")
            );
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct PriceV1 {
    /// Price of the asset (in USD)
    #[serde(rename = "pu")]
    pub price_usd: Decimal256,

    /// Epoch second timestamp of update
    #[serde(rename = "ao")]
    pub as_of_epoch_second: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod unit {
        use std::str::FromStr;

        use super::*;
        use crate::tests::constants::{ASSET_ID_BTC, EPOCH_SECOND_JAN_01_2025};
        use cosmwasm_std::Timestamp;
        use result_extensions::ResultExtensions;

        #[test]
        fn accept_price_update_with_no_timestamp() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: ASSET_ID_BTC.to_owned(),
                usd: Decimal256::from_str("100.123").unwrap(),
                as_of: None,
            };
            assert_eq!(update.validate(current_time), Ok(()));
        }

        #[test]
        fn accept_price_update_with_timestamp() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: ASSET_ID_BTC.to_owned(),
                usd: Decimal256::from_str("100.123").unwrap(),
                as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025).plus_minutes(4)),
            };
            assert_eq!(update.validate(current_time), Ok(()));
        }

        #[test]
        fn reject_if_asset_name_is_empty() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: "".to_owned(),
                usd: Decimal256::from_str("100.123").unwrap(),
                as_of: None,
            };
            assert_eq!(
                update.validate(current_time),
                illegal_argument("Asset name cannot be empty").to_err()
            );
        }

        #[test]
        fn reject_if_usd_price_is_zero() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: ASSET_ID_BTC.to_owned(),
                usd: Decimal256::zero(),
                as_of: None,
            };
            assert_eq!(
                update.validate(current_time),
                illegal_argument(format!("{}: asset price must be > 0", ASSET_ID_BTC)).to_err()
            );
        }

        #[test]
        fn reject_price_update_if_timestamp_is_too_old() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: ASSET_ID_BTC.to_owned(),
                usd: Decimal256::from_str("100.123").unwrap(),
                as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025).minus_days(61)),
            };
            assert_eq!(
                update.validate(current_time),
                illegal_argument("Price timestamp too old").to_err()
            );
        }

        #[test]
        fn reject_price_update_if_timestamp_is_too_far_into_the_future() {
            let current_time = Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025);
            let update = PriceUpdateV1 {
                asset: ASSET_ID_BTC.to_owned(),
                usd: Decimal256::from_str("100.123").unwrap(),
                as_of: Some(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025).plus_days(61)),
            };
            assert_eq!(
                update.validate(current_time),
                illegal_argument("Price timestamp too far into the future").to_err()
            );
        }
    }
}
