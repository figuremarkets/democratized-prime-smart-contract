//! One borrower-published collateral sale. Fill is `Liquidate` with a non-empty
//! subset of this map, before `expires_at`.

use cosmwasm_std::{Decimal256, Timestamp, Uint128, Uint256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct DirectedSaleOfferV1 {
    /// Asset id -> amount (base units). Same keys as collateral maps / Liquidate seize maps.
    pub amounts: BTreeMap<String, Uint128>,
    pub expires_at: Timestamp,
}

/// Drop zero amounts so `{asset: 0}` compares equal to an empty map.
pub fn normalize_seize_map(map: &BTreeMap<String, Uint128>) -> BTreeMap<String, Uint128> {
    map.iter()
        .filter(|(_, amt)| !amt.is_zero())
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

pub fn maps_equal_normalized(a: &BTreeMap<String, Uint128>, b: &BTreeMap<String, Uint128>) -> bool {
    normalize_seize_map(a) == normalize_seize_map(b)
}

/// True when `seize` is a non-empty subset of `offer`: every non-zero seize amount is
/// present on the offer and does not exceed it. Extra keys and over-amounts fail.
pub fn is_seize_within_offer(
    offer: &BTreeMap<String, Uint128>,
    seize: &BTreeMap<String, Uint128>,
) -> bool {
    let seize = normalize_seize_map(seize);
    if seize.is_empty() {
        return false;
    }
    seize
        .iter()
        .all(|(k, amt)| offer.get(k).is_some_and(|have| *amt <= *have))
}

/// Subtract a seize map from an offer. Zeros are dropped. Caller must have already
/// checked [`is_seize_within_offer`]; extra keys are ignored.
pub fn subtract_seize(
    offer: &BTreeMap<String, Uint128>,
    seize: &BTreeMap<String, Uint128>,
) -> BTreeMap<String, Uint128> {
    let seize = normalize_seize_map(seize);
    let mut remaining = offer.clone();
    for (k, amt) in seize {
        let Some(have) = remaining.get(&k).copied() else {
            continue;
        };
        let left = have.u128().saturating_sub(amt.u128());
        if left == 0 {
            remaining.remove(&k);
        } else {
            remaining.insert(k, Uint128::new(left));
        }
    }
    remaining
}

/// Live strictly before the deadline. `now == expires_at` is expired.
pub fn is_live(offer: &DirectedSaleOfferV1, now: Timestamp) -> bool {
    now.nanos() < offer.expires_at.nanos()
}

/// Result of fitting listed amounts under a USD cap.
///
/// [`ScaleToCap::Unchanged`] means a listed amount has no usable value, or the
/// scale overflowed. [`ScaleToCap::Scaled`] is the map to store. It may be empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScaleToCap {
    Unchanged,
    Scaled(BTreeMap<String, Uint128>),
}

/// Clamp each listed amount to the balance still held. A missing or zero balance
/// drops the asset. Zero listed amounts are dropped. Prices are not used.
pub fn clamp_to_balances(
    amounts: &BTreeMap<String, Uint128>,
    balances: &BTreeMap<String, u128>,
) -> BTreeMap<String, Uint128> {
    let mut clamped = BTreeMap::new();
    for (asset_id, amount) in amounts {
        if amount.is_zero() {
            continue;
        }
        let balance = balances.get(asset_id).copied().unwrap_or(0);
        let kept = amount.u128().min(balance);
        if kept > 0 {
            clamped.insert(asset_id.clone(), Uint128::new(kept));
        }
    }
    clamped
}

/// Scale `amounts` so their supplied USD values fit under `cap_usd`.
///
/// `amount_value_usd` is the market USD of each listed amount (already clamped).
/// `offer_usd` is the sum of those values. When `cap_usd` is at least `offer_usd`,
/// the same amounts are returned. Otherwise each amount is
/// `floor(amount * cap_usd / offer_usd)`, and a zero result is dropped.
/// A missing value, a zero value, or an overflow returns [`ScaleToCap::Unchanged`].
pub fn scale_to_cap(
    amounts: &BTreeMap<String, Uint128>,
    amount_value_usd: &BTreeMap<String, Decimal256>,
    cap_usd: Decimal256,
) -> ScaleToCap {
    let mut offer_usd = Decimal256::zero();
    let mut listed: Vec<(&String, u128)> = Vec::new();
    for (asset_id, amount) in amounts {
        if amount.is_zero() {
            continue;
        }
        let Some(value) = amount_value_usd.get(asset_id) else {
            return ScaleToCap::Unchanged;
        };
        if value.is_zero() {
            return ScaleToCap::Unchanged;
        }
        let Some(sum) = offer_usd.checked_add(*value).ok() else {
            return ScaleToCap::Unchanged;
        };
        offer_usd = sum;
        listed.push((asset_id, amount.u128()));
    }
    if listed.is_empty() {
        return ScaleToCap::Scaled(BTreeMap::new());
    }
    if cap_usd >= offer_usd {
        return ScaleToCap::Scaled(
            listed
                .into_iter()
                .map(|(id, amt)| (id.clone(), Uint128::new(amt)))
                .collect(),
        );
    }
    let mut scaled = BTreeMap::new();
    for (asset_id, amount) in listed {
        let Some(kept) = floor_share(amount, cap_usd, offer_usd) else {
            return ScaleToCap::Unchanged;
        };
        if kept > 0 {
            scaled.insert(asset_id.clone(), Uint128::new(kept));
        }
    }
    ScaleToCap::Scaled(scaled)
}

/// `floor(amount * cap_usd / offer_usd)` in the 18-decimal atomics of [`Decimal256`].
fn floor_share(amount: u128, cap_usd: Decimal256, offer_usd: Decimal256) -> Option<u128> {
    let denominator = offer_usd.atomics();
    if denominator.is_zero() {
        return None;
    }
    let numerator = Uint256::from(amount).checked_mul(cap_usd.atomics()).ok()?;
    let quotient = numerator.checked_div(denominator).ok()?;
    quotient.to_string().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::Decimal256;
    use std::str::FromStr;

    fn map(pairs: &[(&str, u128)]) -> BTreeMap<String, Uint128> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Uint128::new(*v)))
            .collect()
    }

    #[test]
    fn normalize_drops_zeros_and_keeps_order() {
        let raw = map(&[("b", 0), ("a", 2), ("c", 0)]);
        assert_eq!(normalize_seize_map(&raw), map(&[("a", 2)]));
        assert!(normalize_seize_map(&map(&[("a", 0)])).is_empty());
    }

    #[test]
    fn maps_equal_ignores_zero_entries() {
        assert!(maps_equal_normalized(
            &map(&[("a", 5), ("b", 0)]),
            &map(&[("a", 5)])
        ));
        assert!(!maps_equal_normalized(&map(&[("a", 5)]), &map(&[("a", 6)])));
    }

    #[test]
    fn seize_within_offer_accepts_subset_and_ignores_zero_seize_entries() {
        let offer = map(&[("a", 5), ("b", 3)]);
        assert!(is_seize_within_offer(&offer, &map(&[("a", 5), ("b", 3)])));
        assert!(is_seize_within_offer(&offer, &map(&[("a", 1)])));
        assert!(is_seize_within_offer(&offer, &map(&[("a", 5), ("b", 0)])));
        assert!(!is_seize_within_offer(&offer, &BTreeMap::new()));
        assert!(!is_seize_within_offer(&offer, &map(&[("a", 0)])));
        assert!(!is_seize_within_offer(&offer, &map(&[("a", 6)])));
        assert!(!is_seize_within_offer(&offer, &map(&[("c", 1)])));
        assert!(!is_seize_within_offer(&offer, &map(&[("a", 1), ("c", 1)])));
    }

    #[test]
    fn subtract_seize_drops_zeros_and_keeps_untouched_assets() {
        let offer = map(&[("a", 5), ("b", 3)]);
        assert_eq!(
            subtract_seize(&offer, &map(&[("a", 2)])),
            map(&[("a", 3), ("b", 3)])
        );
        assert_eq!(subtract_seize(&offer, &map(&[("a", 5)])), map(&[("b", 3)]));
        assert!(subtract_seize(&offer, &map(&[("a", 5), ("b", 3)])).is_empty());
        assert_eq!(
            subtract_seize(&offer, &map(&[("a", 1), ("b", 0)])),
            map(&[("a", 4), ("b", 3)])
        );
    }

    #[test]
    fn is_live_is_strictly_before_deadline() {
        let offer = DirectedSaleOfferV1 {
            amounts: map(&[("a", 1)]),
            expires_at: Timestamp::from_nanos(10),
        };
        assert!(is_live(&offer, Timestamp::from_nanos(9)));
        assert!(!is_live(&offer, Timestamp::from_nanos(10)));
        assert!(!is_live(&offer, Timestamp::from_nanos(11)));
    }

    fn usd(pairs: &[(&str, &str)]) -> BTreeMap<String, Decimal256> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Decimal256::from_str(v).unwrap()))
            .collect()
    }

    fn balances(pairs: &[(&str, u128)]) -> BTreeMap<String, u128> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn scale_to_cap_floors_each_amount_by_the_same_ratio() {
        let amounts = map(&[("a", 100), ("b", 100)]);
        let values = usd(&[("a", "200"), ("b", "300")]);
        assert_eq!(
            scale_to_cap(&amounts, &values, Decimal256::from_str("255").unwrap()),
            ScaleToCap::Scaled(map(&[("a", 51), ("b", 51)]))
        );
    }

    #[test]
    fn scale_to_cap_leaves_amounts_when_cap_covers_the_listing() {
        let amounts = map(&[("a", 100), ("b", 100)]);
        let values = usd(&[("a", "200"), ("b", "300")]);
        assert_eq!(
            scale_to_cap(&amounts, &values, Decimal256::from_str("800").unwrap()),
            ScaleToCap::Scaled(map(&[("a", 100), ("b", 100)]))
        );
    }

    #[test]
    fn scale_to_cap_is_unchanged_when_a_value_is_missing() {
        let amounts = map(&[("a", 100), ("b", 100)]);
        let values = usd(&[("a", "200")]);
        assert_eq!(
            scale_to_cap(&amounts, &values, Decimal256::from_str("255").unwrap()),
            ScaleToCap::Unchanged
        );
    }

    #[test]
    fn scale_to_cap_drops_an_amount_that_floors_to_zero() {
        let amounts = map(&[("a", 1)]);
        let values = usd(&[("a", "100")]);
        assert_eq!(
            scale_to_cap(&amounts, &values, Decimal256::one()),
            ScaleToCap::Scaled(BTreeMap::new())
        );
    }

    #[test]
    fn clamp_to_balances_caps_at_the_held_amount_and_drops_a_missing_balance() {
        let amounts = map(&[("a", 100), ("b", 100)]);
        let held = balances(&[("a", 40)]);
        assert_eq!(clamp_to_balances(&amounts, &held), map(&[("a", 40)]));
    }
}
