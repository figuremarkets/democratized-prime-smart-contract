use crate::model::error::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Coin, Uint128};
use provwasm_std::types::cosmos::base::v1beta1::Coin as ProvCoin;
use result_extensions::ResultExtensions;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum precision allowed: 10^precision must fit in u128 (used in base_to_display).
/// 10^39 > u128::MAX. Provenance does not define a max marker precision; typical assets use 6–18.
pub const MAX_DENOM_PRECISION: u32 = 38;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Denom {
    #[serde(rename = "n")]
    pub name: String,
    #[serde(rename = "p")]
    pub precision: u32,
}

impl Denom {
    pub fn new<S: AsRef<str>, T: Into<u32>>(name: S, precision: T) -> Self {
        Self {
            name: name.as_ref().to_owned(),
            precision: precision.into(),
        }
    }

    pub fn to_cw_coin<T: Into<u128>>(&self, amount: T) -> Coin {
        Coin::new(amount.into(), self.name.clone())
    }

    pub fn to_prov_coin<T: Into<u128>>(&self, amount: T) -> ProvCoin {
        ProvCoin {
            denom: self.name.clone(),
            amount: amount.into().to_string(),
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        ensure!(
            !self.name.trim().is_empty(),
            illegal_argument("Denom name cannot be empty")
        );
        ensure!(
            self.precision <= MAX_DENOM_PRECISION,
            illegal_argument(format!(
                "Denom precision must be <= {}",
                MAX_DENOM_PRECISION
            ))
        );
        Ok(())
    }

    /// Convert amount in base units to a display string (human-readable lending denom amount).
    /// Uses this denom's precision: divides by 10^precision and formats with that many decimal places.
    /// E.g. base 1_500_000 with precision 6 → "1.500000". When precision is 0, returns the whole number only (e.g. "123").
    /// Panics if precision > 38 (10^precision would overflow u128); validation ensures precision ≤ 38.
    pub fn base_to_display(&self, base_units: u128) -> Result<String, ContractError> {
        let divisor = Uint128::new(10u128).checked_pow(self.precision)?;
        let base_units = Uint128::new(base_units);
        let whole = base_units.checked_div(divisor)?;
        let frac = base_units.checked_rem(divisor)?;
        if self.precision == 0 {
            whole.to_string()
        } else if frac == Uint128::zero() {
            format!("{whole}.{}", "0".repeat(self.precision as usize))
        } else {
            format!("{whole}.{:0>width$}", frac, width = self.precision as usize)
        }
        .to_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_to_display_precision_zero_no_trailing_dot() {
        let denom = Denom::new("whole", 0u32);
        assert_eq!(denom.base_to_display(123).unwrap(), "123");
        assert_eq!(denom.base_to_display(0).unwrap(), "0");
    }

    #[test]
    fn base_to_display_precision_six() {
        let denom = Denom::new("uylds.fcc", 6u32);
        assert_eq!(denom.base_to_display(1_500_000).unwrap(), "1.500000");
        assert_eq!(denom.base_to_display(0).unwrap(), "0.000000");
        // Non-zero fractional part (zero-padded)
        assert_eq!(denom.base_to_display(1_000_001).unwrap(), "1.000001");
    }
}
