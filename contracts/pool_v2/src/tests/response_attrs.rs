//! Shared assertions for [`Response`] event attributes in execute tests.

use crate::constants::{
    ATTRIBUTE_BORROW_INDEX, ATTRIBUTE_BORROW_RATE, ATTRIBUTE_LEND_RATE, ATTRIBUTE_LIQUIDITY_INDEX,
    ATTRIBUTE_UTILIZATION,
};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1};
use crate::utils::compute_effective_reserve;
use cosmwasm_std::{Response, Storage, Timestamp};

fn attr_value<'a>(res: &'a Response, key: &str) -> &'a str {
    let keys: Vec<&str> = res.attributes.iter().map(|a| a.key.as_str()).collect();
    res.attributes
        .iter()
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("expected attribute {:?}, found keys {:?}", key, keys))
        .value
        .as_str()
}

fn assert_lend_borrow_rate_attributes_match_expected(
    res: &Response,
    expected_lend: &str,
    expected_borrow: &str,
    expected_li: &str,
    expected_bi: &str,
    expected_util: &str,
) {
    assert_eq!(
        attr_value(res, ATTRIBUTE_LEND_RATE),
        expected_lend,
        "attribute {:?} mismatch",
        ATTRIBUTE_LEND_RATE
    );
    assert_eq!(
        attr_value(res, ATTRIBUTE_BORROW_RATE),
        expected_borrow,
        "attribute {:?} mismatch",
        ATTRIBUTE_BORROW_RATE
    );
    assert_eq!(
        attr_value(res, ATTRIBUTE_LIQUIDITY_INDEX),
        expected_li,
        "attribute {:?} mismatch",
        ATTRIBUTE_LIQUIDITY_INDEX
    );
    assert_eq!(
        attr_value(res, ATTRIBUTE_BORROW_INDEX),
        expected_bi,
        "attribute {:?} mismatch",
        ATTRIBUTE_BORROW_INDEX
    );
    assert_eq!(
        attr_value(res, ATTRIBUTE_UTILIZATION),
        expected_util,
        "attribute {:?} mismatch",
        ATTRIBUTE_UTILIZATION
    );
}

/// Same encoding as [`crate::utils::WithRates::attach_rates`]: loads contract + reserve from `storage`, derives
/// expected `lend_rate`, `borrow_rate`, `liquidity_index`, `borrow_index`, and `utilization` strings
/// (same encoding as tx attributes), and asserts the response includes matching keys and values.
pub fn assert_response_lend_borrow_rates_match_reserve(res: &Response, storage: &dyn Storage) {
    let contract = get_contract_state_v1(storage).expect("contract state");
    let reserve = get_reserve_state_v1(storage).expect("reserve state");
    let (expected_lend, expected_borrow, expected_li, expected_bi, expected_util) =
        crate::utils::lend_borrow_rate_attribute_values(&reserve, &contract.rate_params)
            .expect("expected lend/borrow rate strings from reserve utilization");
    assert_lend_borrow_rate_attributes_match_expected(
        res,
        &expected_lend,
        &expected_borrow,
        &expected_li,
        &expected_bi,
        &expected_util,
    );
}

/// Like [`assert_response_lend_borrow_rates_match_reserve`], but compares to
/// [`crate::utils::compute_effective_reserve`] at `block_time` (for handlers that accrue in memory
/// without persisting, e.g. transfer).
pub fn assert_response_lend_borrow_rates_match_effective_reserve(
    res: &Response,
    storage: &dyn Storage,
    block_time: Timestamp,
) {
    let contract = get_contract_state_v1(storage).expect("contract state");
    let reserve =
        compute_effective_reserve(storage, block_time, &contract.rate_params).expect("effective");
    let (expected_lend, expected_borrow, expected_li, expected_bi, expected_util) =
        crate::utils::lend_borrow_rate_attribute_values(&reserve, &contract.rate_params)
            .expect("expected lend/borrow rate strings from reserve utilization");
    assert_lend_borrow_rate_attributes_match_expected(
        res,
        &expected_lend,
        &expected_borrow,
        &expected_li,
        &expected_bi,
        &expected_util,
    );
}
