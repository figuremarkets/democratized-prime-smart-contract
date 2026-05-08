use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Protocol fee routing mode for splitting borrower interest between suppliers and treasury.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeeModelV1 {
    /// Existing behavior: treasury share = borrower_rate * utilization * reserve_factor.
    #[default]
    ReserveFactor,
    /// Flat spread behavior: treasury share = flat_fee_apr * utilization.
    FlatBorrowSpread,
}

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
    /// Fee mode: reserve-factor split (default) or flat spread from borrower APR.
    #[serde(rename = "fm", default, skip_serializing_if = "is_default_fee_model")]
    pub fee_model: FeeModelV1,
    /// Flat protocol fee APR used when `fee_model = flat_borrow_spread`.
    #[serde(rename = "ff", default, skip_serializing_if = "is_zero_decimal")]
    pub flat_fee_apr: Decimal256,
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
            self.flat_fee_apr < Decimal256::one(),
            illegal_argument("rate_params: flat_fee_apr must be < 1")
        );
        match self.fee_model {
            FeeModelV1::FlatBorrowSpread => {
                ensure!(
                    self.flat_fee_apr <= self.min_rate,
                    illegal_argument(
                        "rate_params: flat_fee_apr must be <= min_rate for flat_borrow_spread mode"
                    )
                );
            }
            FeeModelV1::ReserveFactor => {
                ensure!(
                    self.flat_fee_apr.is_zero(),
                    illegal_argument(
                        "rate_params: flat_fee_apr must be zero when fee_model is reserve_factor"
                    )
                );
            }
        }
        ensure!(
            self.seconds_per_year > 0,
            illegal_argument("rate_params: seconds_per_year must be positive")
        );
        Ok(())
    }
}

fn is_default_fee_model(v: &FeeModelV1) -> bool {
    matches!(v, FeeModelV1::ReserveFactor)
}

fn is_zero_decimal(v: &Decimal256) -> bool {
    v.is_zero()
}
