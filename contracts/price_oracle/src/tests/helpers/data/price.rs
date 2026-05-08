use crate::model::price::PriceV1;
use crate::storage::prices::save_usd_price_v1;
use crate::tests::constants::{DENOM0, EPOCH_SECOND_JAN_01_2025};
use cosmwasm_std::{Decimal256, Storage};
use std::str::FromStr;

// --- PriceV1 helpers ------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceV1Builder {
    pub asset_id: String,
    pub price_usd: Decimal256,
    pub as_of_epoch_second: u64,
}

impl PriceV1Builder {
    #[allow(dead_code)]
    pub fn from(asset_id: String, price: &PriceV1) -> PriceV1Builder {
        PriceV1Builder {
            asset_id: asset_id,
            price_usd: price.price_usd.clone(),
            as_of_epoch_second: price.as_of_epoch_second,
        }
    }
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            asset_id: DENOM0.to_string(),
            price_usd: Decimal256::from_str("0.0111111").unwrap(),
            as_of_epoch_second: EPOCH_SECOND_JAN_01_2025.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn set_asset_id(mut self, asset_id: &str) -> Self {
        self.asset_id = asset_id.to_string();
        self
    }

    #[allow(dead_code)]
    pub fn set_price_usd(mut self, price_usd: &Decimal256) -> Self {
        self.price_usd = price_usd.clone();
        self
    }

    #[allow(dead_code)]
    pub fn set_as_of_time<T: Into<u64>>(mut self, as_of_epoch_second: T) -> Self {
        self.as_of_epoch_second = as_of_epoch_second.into();
        self
    }

    #[allow(dead_code)]
    pub fn build(self) -> (String, PriceV1) {
        (
            self.asset_id,
            PriceV1 {
                price_usd: self.price_usd,
                as_of_epoch_second: self.as_of_epoch_second,
            },
        )
    }

    #[allow(dead_code)]
    pub fn build_and_store(self, store: &mut dyn Storage) -> (String, PriceV1) {
        let (asset_id, price) = self.build();
        save_usd_price_v1(store, asset_id.clone(), &price).unwrap();
        (asset_id, price)
    }
}
