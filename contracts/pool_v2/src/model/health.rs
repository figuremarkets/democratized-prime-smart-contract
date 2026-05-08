//! Borrower health states for LTV-based checks.
//!
//! Only two thresholds are used on-chain: **margin_rate** (must stay at or below to borrow/remove collateral)
//! and **liquidation_rate** (at or above allows liquidation). The "Unhealthy" band between them is
//! the warning zone (no borrow, not yet liquidatable).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub enum BorrowerHealthV1 {
    /// LTV at or below margin rate; can borrow and remove collateral.
    Healthy,

    /// LTV above margin rate but below liquidation rate; cannot borrow, not yet liquidatable.
    Unhealthy,

    /// LTV at or above liquidation rate; position can be liquidated.
    Liquidatable,
}

/// Health value returned by GetBorrowerPosition. Serializes as snake_case string in JSON.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BorrowerHealthResponseV1 {
    /// LTV at or below margin rate.
    Healthy,
    /// LTV above margin rate but below liquidation rate.
    Unhealthy,
    /// LTV at or above liquidation rate.
    Liquidatable,
    /// Borrower has debt but no collateral (oracle/LTV not applicable).
    NoCollateral,
    /// LTV could not be computed (e.g. missing price).
    Unknown,
}

impl From<BorrowerHealthV1> for BorrowerHealthResponseV1 {
    fn from(h: BorrowerHealthV1) -> Self {
        match h {
            BorrowerHealthV1::Healthy => BorrowerHealthResponseV1::Healthy,
            BorrowerHealthV1::Unhealthy => BorrowerHealthResponseV1::Unhealthy,
            BorrowerHealthV1::Liquidatable => BorrowerHealthResponseV1::Liquidatable,
        }
    }
}
