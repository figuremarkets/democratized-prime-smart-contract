//! Tests for SocializeDeficit: contract owner only, pro-rata liquidity index haircut + deficit reduction.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT};
use crate::contract::execute;
use crate::execute::socialize_deficit::{ACTION, ASSERT_OWNER_ERR};
use crate::model::error::ContractError;
use crate::model::OperationalState;
use crate::msg::ExecuteMsg;
use crate::storage::{
    get_contract_state_v1, get_reserve_state_v1, set_contract_state_v1, set_reserve_state_v1,
};
use crate::tests::instantiate_helpers::{setup_instantiated_contract, LENDING_DENOM, OWNER};
use crate::utils::compute_effective_reserve;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{coin, Addr, Decimal256, Uint128};

const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

// --- Auth, validation, operational state ---

#[test]
fn socialize_deficit_non_owner_fails() {
    let (mut deps, env) = setup_instantiated_contract();
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 100;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(10),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn socialize_deficit_fails_when_no_deficit() {
    let (mut deps, env) = setup_instantiated_contract();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(1),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("no deficit"), "message: {}", message);
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn socialize_deficit_fails_when_paused() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 1_000_000;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Paused;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(100),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("paused"), "message: {}", message);
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

// --- Haircut math (liquidity_index + deficit_underlying) ---

#[test]
fn socialize_deficit_haircuts_index_and_reduces_deficit() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 500_000;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let eff_pre =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .expect("effective reserve");
    let l = eff_pre.total_liquidity().expect("total_liquidity");
    let d = Decimal256::from_ratio(Uint128::new(500_000), Uint128::one());
    let scaled = Decimal256::from_ratio(
        Uint128::from(eff_pre.total_scaled_liquidity),
        Uint128::one(),
    );
    let exp_index = l.checked_sub(d).unwrap().checked_div(scaled).unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(500_000),
        },
    )
    .expect("socialize");

    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, ATTRIBUTE_AMOUNT);
    assert_eq!(res.attributes[1].value, "500000");

    let r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(r.deficit_underlying, 0);
    assert_eq!(r.liquidity_index, exp_index);
}

#[test]
fn socialize_deficit_partial_haircut_leaves_deficit() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 1_000_000;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let eff_pre =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .expect("effective reserve");
    let l = eff_pre.total_liquidity().expect("total_liquidity");
    let d = Decimal256::from_ratio(Uint128::new(400_000), Uint128::one());
    let scaled = Decimal256::from_ratio(
        Uint128::from(eff_pre.total_scaled_liquidity),
        Uint128::one(),
    );
    let exp_index = l.checked_sub(d).unwrap().checked_div(scaled).unwrap();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(400_000),
        },
    )
    .expect("partial socialize");

    let r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(r.deficit_underlying, 600_000);
    assert_eq!(r.liquidity_index, exp_index);
}

#[test]
fn socialize_deficit_caps_max_amount_at_remaining_deficit() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 500_000;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let eff_pre =
        compute_effective_reserve(deps.as_ref().storage, env.block.time, &contract.rate_params)
            .expect("effective reserve");
    let l = eff_pre.total_liquidity().expect("total_liquidity");
    let d = Decimal256::from_ratio(Uint128::new(500_000), Uint128::one());
    let scaled = Decimal256::from_ratio(
        Uint128::from(eff_pre.total_scaled_liquidity),
        Uint128::one(),
    );
    let exp_index = l.checked_sub(d).unwrap().checked_div(scaled).unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(10_000_000),
        },
    )
    .expect("max_amount above deficit should cap, not fail");

    assert_eq!(res.attributes[1].value, "500000");

    let r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(r.deficit_underlying, 0);
    assert_eq!(r.liquidity_index, exp_index);
}

// --- Frozen (still allowed) ---

#[test]
fn socialize_deficit_succeeds_when_frozen() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[coin(10_000_000, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend");
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 100;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Frozen;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(100),
        },
    )
    .expect("socialize under frozen");
}

#[test]
fn socialize_deficit_near_total_loss_rejects_zero_index_then_accepts_reduced_amount() {
    let (mut deps, env) = setup_instantiated_contract();
    let scaled = 1_000_000_000_000_000_001u128; // 1e18 + 1
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.total_scaled_liquidity = scaled;
    r.liquidity_index = Decimal256::one();
    r.deficit_underlying = scaled - 1;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();
    let index_before = r.liquidity_index;

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(scaled - 1),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("truncate liquidity_index to zero"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
    let r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(r.liquidity_index, index_before);
    assert_eq!(r.deficit_underlying, scaled - 1);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(scaled - 2),
        },
    )
    .expect("reduced socialization must succeed with non-zero index");

    let r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert!(!r.liquidity_index.is_zero());
    assert_eq!(r.deficit_underlying, 1);

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::Lend {},
    )
    .expect("lend must still work after near-total haircut");
}
