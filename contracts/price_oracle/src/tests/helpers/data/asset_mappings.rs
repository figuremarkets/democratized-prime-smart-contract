use crate::constants::DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS;
use crate::model::asset_mapping::AssetMappingV1;
use crate::storage::asset_mappings::save_asset_mapping_v1;
use cosmwasm_std::Storage;

// --- AssetMappingV1 helpers ------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetMappingV1Builder {
    alt_asset_id: String,
    asset_id: String,
    precision: u32,
    staleness_threshold_seconds: u32,
}

impl AssetMappingV1Builder {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            alt_asset_id: "nbtc.figure.se".to_string(),
            asset_id: "BTC".to_string(),
            precision: 9,
            staleness_threshold_seconds: DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        }
    }

    #[allow(dead_code)]
    pub fn set_alt_asset_id<S: Into<String>>(mut self, alt_asset_id: S) -> Self {
        self.alt_asset_id = alt_asset_id.into();
        self
    }

    #[allow(dead_code)]
    pub fn set_asset_id<S: Into<String>>(mut self, asset_id: S) -> Self {
        self.asset_id = asset_id.into();
        self
    }

    #[allow(dead_code)]
    pub fn set_precision(mut self, precision: u32) -> Self {
        self.precision = precision;
        self
    }

    #[allow(dead_code)]
    pub fn set_staleness_threshold_seconds<T: Into<u32>>(
        mut self,
        staleness_threshold_seconds: T,
    ) -> Self {
        self.staleness_threshold_seconds = staleness_threshold_seconds.into();
        self
    }

    pub fn build(self) -> (String, AssetMappingV1) {
        (
            self.alt_asset_id,
            AssetMappingV1 {
                asset_id: self.asset_id,
                precision: self.precision,
                staleness_threshold_seconds: self.staleness_threshold_seconds,
            },
        )
    }

    pub fn build_and_store(self, store: &mut dyn Storage) -> (String, AssetMappingV1) {
        let (alt_asset_id, mapping) = self.clone().build();
        save_asset_mapping_v1(store, &alt_asset_id, mapping.clone()).unwrap();
        (alt_asset_id, mapping)
    }
}
