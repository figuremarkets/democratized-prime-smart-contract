use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::query::{AssetRequirementV1, ReserveStateResponseV1};
use crate::model::{CollateralAssetV1, ContractStateV1};

/// Response for the GetState query. Uses reserve response DTO (includes total_liquidity / total_borrow).
/// Includes supported collateral (allowed assets and haircuts) and total amounts held so clients get full pool state in one call.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct StateResponseV1 {
    pub contract: ContractStateV1,
    pub reserve: ReserveStateResponseV1,
    /// Supported collateral assets: asset_id and haircut (e.g. 0.8 = 80% of price counts toward LTV). Used for AddCollateral, LTV, liquidation.
    pub supported_collateral: Vec<CollateralAssetV1>,
    /// Total amount of each collateral asset currently held in the pool (sum across all borrowers). Same order as supported_collateral; amount 0 if none held.
    pub total_collateral_held: Vec<AssetRequirementV1>,
}
