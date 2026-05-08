use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Kink interest rate model parameters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct RateParamsV1 {
    #[serde(rename = "tr")]
    pub target_rate: Decimal256,
    #[serde(rename = "minr")]
    pub min_rate: Decimal256,
    #[serde(rename = "maxr")]
    pub max_rate: Decimal256,
    #[serde(rename = "kink")]
    pub kink_utilization: Decimal256,
    #[serde(rename = "rf")]
    pub reserve_factor: Decimal256,
    #[serde(rename = "spy")]
    pub seconds_per_year: u64,
}

impl RateParamsV1 {
    /// Validates rate ordering and bounds. Call at instantiate.
    pub fn validate(&self) -> Result<(), ContractError> {
        ensure!(
            self.min_rate <= self.target_rate,
            illegal_argument("rate_params: min_rate must be <= target_rate")
        );
        ensure!(
            self.target_rate <= self.max_rate,
            illegal_argument("rate_params: target_rate must be <= max_rate")
        );
        ensure!(
            self.max_rate <= Decimal256::one(),
            illegal_argument("rate_params: max_rate must be <= 1 (100% APR)")
        );
        ensure!(
            !self.kink_utilization.is_zero() && self.kink_utilization < Decimal256::one(),
            illegal_argument("rate_params: kink_utilization must be in (0, 1)")
        );
        ensure!(
            self.reserve_factor < Decimal256::one(),
            illegal_argument("rate_params: reserve_factor must be < 1")
        );
        ensure!(
            self.seconds_per_year > 0,
            illegal_argument("rate_params: seconds_per_year must be positive")
        );
        Ok(())
    }
}
