use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, Serializer};

/// Protocol fee routing mode for splitting borrower interest between suppliers and treasury.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeeModelV1 {
    /// Default behavior: treasury share = borrower_rate * utilization * reserve_factor.
    #[default]
    ReserveFactor,
    /// Flat spread behavior: treasury share = flat_fee_apr * utilization.
    FlatBorrowSpread,
}

/// A 365-day year, in seconds, for linear accrual.
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

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
    /// The unused fee field for the inactive mode must be zero so a later mode switch
    /// cannot revive a stale value from a full payload.
    #[serde(rename = "fm", default, skip_serializing_if = "is_default_fee_model")]
    pub fee_model: FeeModelV1,
    /// Flat protocol fee APR used when `fee_model = flat_borrow_spread`.
    #[serde(rename = "ff", default, skip_serializing_if = "is_zero_decimal")]
    pub flat_fee_apr: Decimal256,
    /// Fixed at 31536000. Optional on input; must equal 31536000 if provided. Retained so responses keep `spy`.
    /// Responses report 31536000 even when storage holds a different year.
    #[serde(
        rename = "spy",
        default = "default_seconds_per_year",
        serialize_with = "serialize_seconds_per_year"
    )]
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
                ensure!(
                    self.reserve_factor.is_zero(),
                    illegal_argument(
                        "rate_params: reserve_factor must be zero when fee_model is flat_borrow_spread"
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
            self.seconds_per_year == SECONDS_PER_YEAR,
            illegal_argument("rate_params: seconds_per_year must be 31536000")
        );
        Ok(())
    }
}

fn default_seconds_per_year() -> u64 {
    SECONDS_PER_YEAR
}

/// Responses advertise the accrual constant, not a stale stored year.
/// Tests that check spy rejection must send raw JSON or pass the struct straight to `execute()`,
/// because serializing a `RateParamsV1` always writes 31536000 and would hide the bad value.
fn serialize_seconds_per_year<S>(_: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(SECONDS_PER_YEAR)
}

fn is_default_fee_model(v: &FeeModelV1) -> bool {
    matches!(v, FeeModelV1::ReserveFactor)
}

fn is_zero_decimal(v: &Decimal256) -> bool {
    v.is_zero()
}
