use crate::model::error::ContractError;
use cosmwasm_std::{Decimal256, Timestamp, Uint128, Uint256};
use result_extensions::ResultExtensions;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reserve state: indexes, aggregate scaled lend/borrow, and protocol reserve accrual.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ReserveStateV1 {
    #[serde(rename = "li")]
    pub liquidity_index: Decimal256,
    #[serde(rename = "bi")]
    pub borrow_index: Decimal256,
    #[serde(rename = "lu")]
    pub last_updated_at: Timestamp,
    #[serde(rename = "tsl")]
    pub total_scaled_liquidity: u128,
    #[serde(rename = "tsb")]
    pub total_scaled_borrow: u128,
    /// Protocol share of interest (reserve factor), in lending base units. Updated when indexes accrue.
    #[serde(rename = "ar", default)]
    pub accrued_reserve: u128,
    /// Bad debt / insolvency shortfall booked in lending base units (not scaled). Does not accrue
    /// borrower interest; reduces effective cash alongside borrows. Default 0 for legacy store.
    #[serde(rename = "du", default)]
    pub deficit_underlying: u128,
}

impl ReserveStateV1 {
    /// Total supplied (underlying): sum of all lenders' balances.
    pub fn total_liquidity(&self) -> Result<Decimal256, ContractError> {
        let scaled = Decimal256::from_atomics(Uint256::from(self.total_scaled_liquidity), 0)
            .unwrap_or(Decimal256::zero());
        scaled.checked_mul(self.liquidity_index).map_err(Into::into)
    }

    /// Total borrowed (underlying): sum of all borrowers' debt.
    pub fn total_borrow(&self) -> Result<Decimal256, ContractError> {
        let scaled = Decimal256::from_atomics(Uint256::from(self.total_scaled_borrow), 0)
            .unwrap_or(Decimal256::zero());
        scaled.checked_mul(self.borrow_index).map_err(Into::into)
    }

    /// Cash available = total_liquidity - total_borrow - deficit_underlying (underlying units).
    /// `deficit_underlying` is the insolvency shortfall booked on bad-debt liquidation (lending base units).
    /// Uses saturating_sub so pathological state clamps to zero.
    pub fn cash(&self) -> Result<Decimal256, ContractError> {
        let deficit =
            Decimal256::from_ratio(Uint128::from(self.deficit_underlying), Uint128::one());
        self.total_liquidity()?
            .saturating_sub(self.total_borrow()?)
            .saturating_sub(deficit)
            .to_ok()
    }

    /// Utilization = total_borrow / (cash + total_borrow) = total_borrow / total_liquidity.
    /// Same as spreadsheet: "Total borrows / (Cash available + Total borrows)".
    pub fn utilization(&self) -> Result<Decimal256, ContractError> {
        let liquidity = self.total_liquidity()?;
        if liquidity.is_zero() {
            return Decimal256::zero().to_ok();
        }
        self.total_borrow()?
            .checked_div(liquidity)
            .map_err(Into::into)
    }
}
