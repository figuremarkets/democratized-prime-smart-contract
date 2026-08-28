use crate::common::{illegal_argument, ContractError};
use cosmwasm_std::{ensure, Decimal256, Timestamp, Uint128, Uint256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[allow(deprecated)]
pub struct AssetPriceResponseV1 {
    /// Scaled per-base-unit USD (`display_price_usd / 10^precision`).
    ///
    /// **Deprecated:** still serialized for older clients that multiply
    /// `price_usd × amount`. That floors cheap high-precision prices to zero.
    /// New consumers must use [`Self::display_price_usd`] and [`Self::precision`],
    /// or [`Self::value_usd`].
    #[deprecated(note = "use display_price_usd and precision, or AssetPriceResponseV1::value_usd")]
    pub price_usd: Decimal256,

    /// Unscaled display USD price stored by the oracle (e.g. $ per BTC).
    /// Together with [`Self::precision`]: notional = `display_price_usd * amount / 10^precision`.
    /// Defaults to zero when deserializing older payloads that omit this field.
    #[serde(default)]
    pub display_price_usd: Decimal256,

    /// Mapping precision for the requested asset id (0 = identity / display asset).
    /// Defaults to 0 when deserializing older payloads that omit this field.
    #[serde(default)]
    pub precision: u32,

    /// Epoch second timestamp of update
    pub as_of_epoch_second: u64,

    /// The expiration time of the price in epoch seconds. This is defined as the
    /// [`AssetPriceResponseV1::as_of_epoch_second`] + the staleness threshold for the asset.
    pub expiration_epoch_seconds: u64,
}

impl AssetPriceResponseV1 {
    /// Identity-mapped price: display USD is already per-base-unit (precision 0).
    /// Still fills deprecated [`Self::price_usd`] for older clients.
    #[allow(deprecated)]
    pub fn new(
        price_usd: Decimal256,
        as_of_epoch_second: u64,
        expiration_epoch_seconds: u64,
    ) -> Self {
        Self {
            price_usd,
            display_price_usd: price_usd,
            precision: 0,
            as_of_epoch_second,
            expiration_epoch_seconds,
        }
    }

    /// Display USD and mapping precision used for notional math.
    ///
    /// Prefer `display_price_usd` when the oracle populated it. If a consumer
    /// deserialized an older payload (fields default to zero), fall back to the
    /// deprecated scaled `price_usd` at precision 0 — the historical
    /// "already per-base-unit" meaning.
    #[allow(deprecated)]
    fn valuation_inputs(&self) -> (Decimal256, u32) {
        if self.display_price_usd.is_zero() && self.precision == 0 {
            (self.price_usd, 0)
        } else {
            (self.display_price_usd, self.precision)
        }
    }

    /// True when the valuation display price is zero (unusable for LTV / seize math).
    pub fn is_zero_price(&self) -> bool {
        self.valuation_inputs().0.is_zero()
    }

    /// USD notional of `amount` base units: `display_price_usd * amount / 10^precision`.
    ///
    /// Computed in integer atomics so a cheap display price at high precision does not
    /// floor to zero the way multiplying deprecated [`Self::price_usd`] by amount does.
    pub fn value_usd(&self, amount: u128) -> Result<Decimal256, ContractError> {
        let (display, precision) = self.valuation_inputs();
        if amount == 0 {
            return Ok(Decimal256::zero());
        }
        ensure!(!display.is_zero(), illegal_argument("Asset price is zero"));
        let numerator = display.atomics().checked_mul(Uint256::from(amount))?;
        let exp = Decimal256::DECIMAL_PLACES
            .checked_add(precision)
            .ok_or_else(|| illegal_argument("Price precision overflow"))?;
        let denominator = Uint256::from(10u64).checked_pow(exp)?;
        Decimal256::checked_from_ratio(numerator, denominator).map_err(Into::into)
    }

    /// Ceil of the base-unit amount whose notional is at least `value` USD:
    /// `ceil(value * 10^precision / display_price_usd)`.
    pub fn amount_from_usd(&self, value: Decimal256) -> Result<u128, ContractError> {
        let (display, precision) = self.valuation_inputs();
        if value.is_zero() {
            return Ok(0);
        }
        ensure!(!display.is_zero(), illegal_argument("Asset price is zero"));
        let scale = Uint256::from(10u64).checked_pow(precision)?;
        let numerator = value.atomics().checked_mul(scale)?;
        let denominator = display.atomics();
        let quot = numerator.checked_div(denominator)?;
        let rem = numerator.checked_rem(denominator)?;
        let ceil = if rem.is_zero() {
            quot
        } else {
            quot.checked_add(Uint256::one())?
        };
        Uint128::try_from(ceil)
            .map(|u| u.u128())
            .map_err(Into::into)
    }

    /// Tests if the price is considered stale by comparing the current time
    /// against [`AssetPriceResponseV1::expiration_epoch_seconds`].
    pub fn is_stale(&self, at: Timestamp) -> bool {
        at.seconds() >= self.expiration_epoch_seconds
    }

    pub fn expired_at(&self) -> Timestamp {
        Timestamp::from_seconds(self.expiration_epoch_seconds)
    }
}

/// Map of denom to PriceV1 record
/// K: denom
/// V: price & metadata of asset
pub type PriceMapResponse = HashMap<String, AssetPriceResponseV1>;

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal256 {
        Decimal256::from_str(s).unwrap()
    }

    #[test]
    fn value_usd_precision_0_is_display_times_amount() {
        let p = AssetPriceResponseV1::new(dec("100"), 0, 1);
        assert_eq!(p.value_usd(2).unwrap(), dec("200"));
    }

    #[test]
    fn value_usd_nbtc_one_whole_token() {
        let p = AssetPriceResponseV1 {
            price_usd: dec("0.000100000123"),
            display_price_usd: dec("100000.123"),
            precision: 9,
            as_of_epoch_second: 0,
            expiration_epoch_seconds: 1,
        };
        assert_eq!(p.value_usd(1_000_000_000).unwrap(), dec("100000.123"));
    }

    #[test]
    fn value_usd_half_dollar_18_decimals_one_whole_token() {
        // Scaled `price_usd` would be 0 at precision 18; integer notional is $0.50.
        let p = AssetPriceResponseV1 {
            price_usd: Decimal256::zero(),
            display_price_usd: dec("0.5"),
            precision: 18,
            as_of_epoch_second: 0,
            expiration_epoch_seconds: 1,
        };
        assert_eq!(p.value_usd(1_000_000_000_000_000_000).unwrap(), dec("0.5"));
    }

    #[test]
    fn amount_from_usd_inverts_value_usd_at_precision_18() {
        let p = AssetPriceResponseV1 {
            price_usd: Decimal256::zero(),
            display_price_usd: dec("0.5"),
            precision: 18,
            as_of_epoch_second: 0,
            expiration_epoch_seconds: 1,
        };
        assert_eq!(
            p.amount_from_usd(dec("0.5")).unwrap(),
            1_000_000_000_000_000_000
        );
    }

    #[test]
    fn legacy_json_falls_back_to_scaled_price_usd() {
        let json = r#"{"price_usd":"1.5","as_of_epoch_second":1,"expiration_epoch_seconds":2}"#;
        let p: AssetPriceResponseV1 = serde_json::from_str(json).unwrap();
        assert!(p.display_price_usd.is_zero());
        assert_eq!(p.precision, 0);
        assert_eq!(p.value_usd(2).unwrap(), dec("3"));
    }
}
