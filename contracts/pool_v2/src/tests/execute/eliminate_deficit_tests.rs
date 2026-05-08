//! Tests for EliminateDeficit: contract owner **`accrued_reserve`** vs open **`bank`** funding, partial clearance,
//! bank refund, non-owner rules, no deficit, paused, wrong funds, and nothing to apply from accrued.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT};
use crate::contract::execute;
use crate::execute::eliminate_deficit::ACTION;
use crate::model::error::ContractError;
use crate::model::OperationalState;
use crate::msg::execute::EliminateDeficitFunding;
use crate::msg::ExecuteMsg;
use crate::storage::{
    get_contract_state_v1, get_reserve_state_v1, set_contract_state_v1, set_reserve_state_v1,
};
use crate::tests::instantiate_helpers::{setup_instantiated_contract, LENDING_DENOM, OWNER};
use cosmwasm_std::testing::{message_info, MockApi};
use cosmwasm_std::{coin, Addr, BankMsg, CosmosMsg, MemoryStorage, OwnedDeps, Uint128};

const OTHER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

fn set_reserve_deficit_and_accrued(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    deficit_underlying: u128,
    accrued_reserve: u128,
) {
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = deficit_underlying;
    r.accrued_reserve = accrued_reserve;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();
}

#[test]
fn eliminate_deficit_non_owner_fails_for_accrued_reserve() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 100, 50);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(10),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::NotAuthorizedError { .. } => {}
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn eliminate_deficit_fails_when_no_deficit() {
    let (mut deps, env) = setup_instantiated_contract();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(1_000_000),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("no deficit"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn eliminate_deficit_fails_when_paused() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 10, 10);
    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Paused;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(10),
            },
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

#[test]
fn eliminate_deficit_succeeds_when_frozen_accrued_mode() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 40, 100);
    let mut c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    c.operational_state = OperationalState::Frozen;
    set_contract_state_v1(deps.as_mut().storage, &c).unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(40),
            },
        },
    )
    .expect("eliminate_deficit under frozen");

    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);
    assert_eq!(reserve.accrued_reserve, 60);
}

#[test]
fn eliminate_deficit_accrued_partial_clears_deficit_and_accrued() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 100, 50);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(40),
            },
        },
    )
    .expect("accrued partial");

    let cleared: u128 = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_AMOUNT)
        .expect("amount attr")
        .value
        .parse()
        .unwrap();
    assert_eq!(cleared, 40);
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 60);
    assert_eq!(reserve.accrued_reserve, 10);
    assert!(res.messages.is_empty());
}

#[test]
fn eliminate_deficit_bank_refunds_unapplied_lending() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 100, 0);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(300, LENDING_DENOM)]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::Bank {
                max_underlying: Uint128::new(1_000_000),
            },
        },
    )
    .expect("bank mode");

    let applied: u128 = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_AMOUNT)
        .unwrap()
        .value
        .parse()
        .unwrap();
    assert_eq!(applied, 100);
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
            assert_eq!(to_address.as_str(), OWNER);
            assert_eq!(amount.len(), 1);
            assert_eq!(amount[0].denom, LENDING_DENOM);
            assert_eq!(amount[0].amount.u128(), 200, "refund sent - applied");
        }
        _ => panic!("expected Bank refund, got {:?}", res.messages[0].msg),
    }
}

#[test]
fn eliminate_deficit_bank_non_owner_succeeds() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 80, 0);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OTHER), &[coin(80, LENDING_DENOM)]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::Bank {
                max_underlying: Uint128::new(1_000_000),
            },
        },
    )
    .expect("non-owner bank eliminate");

    let applied: u128 = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_AMOUNT)
        .unwrap()
        .value
        .parse()
        .unwrap();
    assert_eq!(applied, 80);
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.deficit_underlying, 0);
    assert!(res.messages.is_empty(), "exact pay-in: no refund");
}

#[test]
fn eliminate_deficit_accrued_fails_with_attached_funds() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 10, 10);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(5),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn eliminate_deficit_accrued_fails_when_nothing_to_clear_from_accrued() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 100, 0);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(50),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("accrued_reserve") || message.contains("Nothing to clear"),
                "message: {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn eliminate_deficit_fails_max_underlying_zero() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 10, 10);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::zero(),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("max_underlying"), "message: {}", message);
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn eliminate_deficit_bank_fails_when_nothing_applied() {
    let (mut deps, env) = setup_instantiated_contract();
    set_reserve_deficit_and_accrued(&mut deps, 100, 0);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::Bank {
                max_underlying: Uint128::new(50),
            },
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } | ContractError::IllegalArgumentError { .. } => {}
        _ => panic!("expected funds or amount error, got {:?}", err),
    }
}
