use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct CollateralAssetV1 {
    #[serde(rename = "id")]
    pub asset_id: String,
    /// Discount for valuation: collateral value = price × amount × haircut. None = 100% (no discount; full value).
    #[serde(rename = "h")]
    pub haircut: Option<Decimal256>,
}

impl CollateralAssetV1 {
    pub fn validate(&self) -> Result<(), ContractError> {
        ensure!(
            !self.asset_id.trim().is_empty(),
            illegal_argument("Asset ID cannot be empty")
        );
        if let Some(h) = self.haircut {
            ensure!(!h.is_zero(), illegal_argument("Haircut must be > 0"));
            ensure!(
                h <= Decimal256::percent(100),
                illegal_argument("Haircut cannot exceed 100%")
            );
        }
        Ok(())
    }
}

/// Returns the asset's haircut from the supported list, or 100% if not found (no discount; value = price × quantity).
pub fn haircut_percentage(supported_assets: &[CollateralAssetV1], asset_id: &str) -> Decimal256 {
    supported_assets
        .iter()
        .find(|a| a.asset_id == asset_id)
        .and_then(|a| a.haircut)
        .unwrap_or(Decimal256::percent(100))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
pub struct BorrowerCollateralV1 {
    #[serde(rename = "a")]
    pub amounts: BTreeMap<String, u128>,
}
