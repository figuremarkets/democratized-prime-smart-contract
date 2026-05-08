//! Tests for GetReserve query.

use crate::contract::query;
use crate::model::ReserveStateV1;
use crate::msg::QueryMsg;
use crate::storage::{get_reserve_state_v1, set_reserve_state_v1};
use crate::tests::query::common::setup_instantiated;
use cosmwasm_std::from_json;
use cosmwasm_std::Decimal256;
use std::str::FromStr;

#[test]
fn get_reserve_returns_reserve_rates_and_utilization() {
    let (deps, env) = setup_instantiated();
    let bin = query(deps.as_ref(), env, QueryMsg::GetReserve {}).expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetReserve response");
    assert!(res.get("reserve").is_some());
    assert!(res.get("current_borrower_rate").is_some());
    assert!(res.get("current_lender_rate").is_some());
    assert!(res.get("utilization").is_some());
    let util: &str = res["utilization"].as_str().unwrap();
    assert_eq!(util, "0");
}

#[test]
fn get_reserve_returns_rates_and_utilization_when_reserve_has_usage() {
    let (mut deps, env) = setup_instantiated();
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let used = ReserveStateV1 {
        liquidity_index: Decimal256::from_str("1.05").unwrap(),
        borrow_index: Decimal256::from_str("1.02").unwrap(),
        last_updated_at: env.block.time,
        total_scaled_liquidity: 100_000,
        total_scaled_borrow: 50_000,
        ..reserve
    };
    set_reserve_state_v1(deps.as_mut().storage, &used).expect("set reserve");
    let bin = query(deps.as_ref(), env, QueryMsg::GetReserve {}).expect("query should succeed");
    let res: serde_json::Value = from_json(bin).expect("decode GetReserve response");
    assert!(res.get("reserve").is_some());
    let util: &str = res["utilization"].as_str().unwrap();
    let util_d = Decimal256::from_str(util).unwrap();
    assert!(
        util_d > Decimal256::zero() && util_d < Decimal256::one(),
        "utilization should be in (0,1)"
    );
    let borrower_rate: &str = res["current_borrower_rate"].as_str().unwrap();
    let lender_rate: &str = res["current_lender_rate"].as_str().unwrap();
    assert!(!borrower_rate.is_empty());
    assert!(!lender_rate.is_empty());
}
