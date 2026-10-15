//! One borrower-published collateral sale. Fill is `Liquidate` with a non-empty
//! subset of this map, before `expires_at`.

use cosmwasm_std::{Timestamp, Uint128};
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
