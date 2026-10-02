//! One borrower-published collateral sale. Fill is `Liquidate` with this exact map, before `expires_at`.

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
