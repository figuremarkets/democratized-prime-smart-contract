//! Tests for GetState query.

use crate::contract::query;
use crate::model::{ReserveStateV1, StateResponseV1};
use crate::msg::QueryMsg;
use crate::storage::{get_reserve_state_v1, set_reserve_state_v1};
use crate::tests::query::common::{setup_instantiated, REPO_TOKEN_CW20};
use cosmwasm_std::Decimal256;
use cosmwasm_std::{from_json, Uint128};
use std::str::FromStr;

#[test]
fn get_state_returns_contract_and_effective_reserve() {
    let (deps, env) = setup_instantiated();
    let bin = query(deps.as_ref(), env, QueryMsg::GetState {}).expect("query should succeed");
    let state: StateResponseV1 = from_json(bin).expect("decode GetState response");
    assert_eq!(
        state
            .contract
            .repo_token_cw20_address
            .as_ref()
            .map(|a| a.as_str()),
        Some(REPO_TOKEN_CW20)
    );
    assert_eq!(state.supported_collateral.len(), 1);
    assert_eq!(state.supported_collateral[0].asset_id, "asset.one");
    assert!(state.supported_collateral[0].haircut.is_some());
    assert_eq!(state.total_collateral_held.len(), 1);
    assert_eq!(state.total_collateral_held[0].asset_id, "asset.one");
    assert_eq!(state.total_collateral_held[0].amount, Uint128::zero());
    let reserve = ReserveStateV1::from(state.reserve);
    assert_eq!(reserve.total_scaled_liquidity, 0);
    assert_eq!(reserve.total_scaled_borrow, 0);
    assert_eq!(reserve.liquidity_index, Decimal256::one());
    assert_eq!(reserve.borrow_index, Decimal256::one());
}

#[test]
fn get_state_returns_accrued_reserve_after_usage() {
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
    let bin = query(deps.as_ref(), env, QueryMsg::GetState {}).expect("query should succeed");
    let state: StateResponseV1 = from_json(bin).expect("decode GetState response");
    let decoded = ReserveStateV1::from(state.reserve);
    assert_eq!(decoded.liquidity_index, used.liquidity_index);
    assert_eq!(decoded.borrow_index, used.borrow_index);
    assert_eq!(decoded.total_scaled_liquidity, 100_000);
    assert_eq!(decoded.total_scaled_borrow, 50_000);
}
