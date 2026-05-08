//! Query response types for pool_v2 contract queries.
//!
//! We do not JSON-serialize storage models (e.g. ReserveStateV1) directly. Response types
//! include derived fields (total_liquidity, total_borrow) so clients get a complete view.

use crate::model::error::ContractError;
use crate::model::{Denom, ReserveStateV1};
use cosmwasm_std::{Timestamp, Uint128};
use result_extensions::ResultExtensions;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct AssetRequirementV1 {
    pub asset_id: String,
    pub amount: Uint128,
}

/// Reserve as returned in GetState / GetReserve: stored fields plus derived totals (strings for JSON).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ReserveStateResponseV1 {
    pub liquidity_index: String,
    pub borrow_index: String,
    pub last_updated_at: Timestamp,
    pub total_scaled_liquidity: String,
    pub total_scaled_borrow: String,
    pub accrued_reserve: String,
    /// Bad debt shortfall in lending base units (see liquidation bad-debt path).
    pub deficit_underlying: String,
    /// Total supplied (underlying): total_scaled_liquidity × liquidity_index.
    pub total_liquidity: String,
    /// Total borrowed (underlying): total_scaled_borrow × borrow_index.
    pub total_borrow: String,
}

impl TryFrom<ReserveStateV1> for ReserveStateResponseV1 {
    type Error = ContractError;

    fn try_from(r: ReserveStateV1) -> Result<Self, Self::Error> {
        let total_liquidity = r.total_liquidity()?;
        let total_borrow = r.total_borrow()?;
        Self {
            liquidity_index: r.liquidity_index.to_string(),
            borrow_index: r.borrow_index.to_string(),
            last_updated_at: r.last_updated_at,
            total_scaled_liquidity: r.total_scaled_liquidity.to_string(),
            total_scaled_borrow: r.total_scaled_borrow.to_string(),
            accrued_reserve: r.accrued_reserve.to_string(),
            deficit_underlying: r.deficit_underlying.to_string(),
            total_liquidity: total_liquidity.to_string(),
            total_borrow: total_borrow.to_string(),
        }
        .to_ok()
    }
}

impl From<ReserveStateResponseV1> for ReserveStateV1 {
    fn from(r: ReserveStateResponseV1) -> Self {
        use cosmwasm_std::Decimal256;
        Self {
            liquidity_index: Decimal256::from_str(&r.liquidity_index).unwrap_or(Decimal256::zero()),
            borrow_index: Decimal256::from_str(&r.borrow_index).unwrap_or(Decimal256::zero()),
            last_updated_at: r.last_updated_at,
            total_scaled_liquidity: r.total_scaled_liquidity.parse().unwrap_or(0),
            total_scaled_borrow: r.total_scaled_borrow.parse().unwrap_or(0),
            accrued_reserve: r.accrued_reserve.parse().unwrap_or(0),
            deficit_underlying: r.deficit_underlying.parse().unwrap_or(0),
        }
    }
}

/// Response for the GetReserve query.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ReserveResponseV1 {
    /// Effective reserve (indexes accrued to current block) with total_liquidity / total_borrow.
    pub reserve: ReserveStateResponseV1,
    /// Current borrower APR (from utilization).
    pub current_borrower_rate: String,
    /// Current lender APR (from utilization).
    pub current_lender_rate: String,
    /// Utilization (total_borrow / total_liquidity).
    pub utilization: String,
}

/// Response for the GetBorrowerPosition query: debt plus collateral amounts, value (USD), LTV, and health.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct BorrowerPositionResponseV1 {
    pub address: String,
    pub scaled_borrow: String,
    pub underlying_debt: String,
    pub underlying_debt_display: String,
    pub lending_denom: Denom,
    /// Collateral amounts per asset (base units).
    pub collateral: Vec<AssetRequirementV1>,
    /// Total collateral value in USD (after haircuts). "0" if no collateral or oracle unavailable.
    pub collateral_value_usd: String,
    /// Loan-to-value (debt value / collateral value). "0" if no collateral.
    pub loan_to_value: String,
    /// Health state (serializes as "healthy" | "unhealthy" | "liquidatable" | "no_collateral" | "unknown").
    pub health: crate::model::health::BorrowerHealthResponseV1,
    /// When health is "unknown", the reason LTV/health could not be computed (e.g. missing oracle price).
    pub health_unknown_reason: Option<String>,
}

/// Response for the GetCollateralRequirements query.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct CollateralRequirementsResponseV1 {
    /// Required total collateral value in USD (after haircuts) so LTV <= margin_rate (Healthy; covers existing + new debt).
    pub required_collateral_value_usd: String,
    /// Additional collateral value in USD the user must add. Equals required total when no borrower; when borrower is set, total minus their existing collateral. Use this to combine assets (any mix whose haircutted value ≥ this).
    pub additional_collateral_value_usd: String,
    /// Per-asset minimum amount. When borrower is set these are *additional* amounts needed; otherwise the amount of each asset that would satisfy the full requirement alone.
    pub required: Vec<AssetRequirementV1>,
}

/// Response for the GetLenderStatus query. Supply balance comes from repo_token_cw20 Balance (and TokenInfo) query.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct LenderStatusResponseV1 {
    /// When true, this lender must pass commit_funds: true in Withdraw/WithdrawExact payloads to withdraw.
    pub require_commit_on_exit: bool,
}
