//! Tests for operational state: Active, Frozen, Paused.
//! Frozen blocks Lend and Borrow; Paused is full freeze (only owner config; Liquidate, Repay,
//! Withdraw, WithdrawReserve, AddCollateral, and RemoveCollateral are blocked).
//! When Frozen, Receive (CW20) with Withdraw/WithdrawExact/Transfer/TransferExact is still allowed.
//! SetOperationalState accepts no funds.

use crate::contract::execute;
use crate::execute::set_operational_state::ASSERT_OWNER_ERR;
use crate::model::error::{illegal_argument, ContractError};
use crate::model::OperationalState;
use crate::msg::execute::Cw20ReceivePayload;
use crate::msg::ExecuteMsg;
use crate::storage::{get_contract_state_v1, get_reserve_state_v1};
use crate::tests::instantiate_helpers::{
    setup_instantiated_contract, LENDER, LENDING_DENOM, OWNER,
};
use crate::utils::underlying_to_scaled_liquidity;
use cosmwasm_std::testing::{message_info, MockApi};
use cosmwasm_std::{coin, to_json_binary, Addr, Uint128};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use cw20::Cw20ReceiveMsg;
use cw_ownable::{get_ownership, Action};
use std::collections::BTreeMap;

fn setup() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    setup_instantiated_contract()
}

#[test]
fn set_operational_state_fails_with_funds() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn get_state_returns_operational_state_active_after_instantiate() {
    let (deps, _env) = setup();
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.operational_state, OperationalState::Active);
}

#[test]
fn frozen_blocks_lend() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    )
    .expect("set frozen should succeed");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(100_000_000u128, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("frozen") && message.contains("lend"),
                "expected frozen/lend message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn frozen_blocks_borrow() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    )
    .expect("set frozen should succeed");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("borrower"), &[]),
        ExecuteMsg::Borrow {
            amount: Uint128::new(1_000_000),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("frozen") && message.contains("borrow"),
                "expected frozen/borrow message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn paused_blocks_lend() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused should succeed");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(100_000_000u128, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "expected paused message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn paused_allows_set_operational_state() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    // The contract owner can set back to Active while paused
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Active,
        },
    )
    .expect("set active should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.operational_state, OperationalState::Active);
}

#[test]
fn paused_blocks_repay() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(1_000_000u128, LENDING_DENOM)],
        ),
        ExecuteMsg::Repay {},
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "Repay when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn paused_blocks_withdraw() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::Withdraw {
            lender: LENDER.to_string(),
            amount: None,
            commit_funds: None,
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "Withdraw when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn paused_blocks_withdraw_reserve() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::WithdrawReserve { recipient: None },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "WithdrawReserve when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn paused_blocks_liquidate() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let collateral_to_seize = BTreeMap::new();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(
            &Addr::unchecked(OWNER),
            &[coin(1_000_000u128, LENDING_DENOM)],
        ),
        ExecuteMsg::Liquidate {
            borrower: "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu".to_string(),
            collateral_to_seize,
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "Liquidate when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn paused_blocks_add_collateral() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked("borrower"),
            &[coin(1_000_000u128, LENDING_DENOM)],
        ),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "AddCollateral when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn paused_blocks_remove_collateral() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    let to_remove = BTreeMap::from([("somedenom".to_string(), Uint128::new(1))]);
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("borrower"), &[]),
        ExecuteMsg::RemoveCollateral { to_remove },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("paused"),
                "RemoveCollateral when paused should return pause error, got {}",
                message
            );
        }
        _ => panic!("expected IllegalStateError with paused, got {:?}", err),
    }
}

#[test]
fn set_operational_state_non_owner_fails() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(LENDER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

/// Valid Provenance bech32 for receive handler addr_validate (lend_tests LENDER may be shorthand).
const LENDER_BECH32: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";

/// When Frozen, Receive with Withdraw payload must still succeed (payload-level guard allows it).
#[test]
fn frozen_allows_receive_withdraw() {
    let (mut deps, env) = setup();
    let lend_amount = 100_000_000u128;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(
            &Addr::unchecked(LENDER),
            &[coin(lend_amount, LENDING_DENOM)],
        ),
        ExecuteMsg::Lend {},
    )
    .expect("lend");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    )
    .expect("set frozen");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let repo_addr = contract.repo_token_cw20_address.expect("repo token bound");
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let withdraw_underlying = 10_000_000u128;
    let scaled_to_remove =
        underlying_to_scaled_liquidity(withdraw_underlying, reserve.liquidity_index).unwrap();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&repo_addr, &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: LENDER_BECH32.to_string(),
            amount: Uint128::from(scaled_to_remove),
            msg: to_json_binary(&Cw20ReceivePayload::Withdraw {
                amount: Uint128::new(withdraw_underlying),
                commit_funds: None,
            })
            .unwrap(),
        }),
    )
    .expect("Receive(Withdraw) should succeed when Frozen");

    assert!(
        !res.messages.is_empty(),
        "withdraw should emit burn/send messages"
    );
}

const NEW_OWNER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
/// Second valid `tp` bech32 for ownership tests (distinct from [`NEW_OWNER`]).
const SECOND_NEW_OWNER: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";

#[test]
fn update_ownership_transfer_succeeds_after_accept() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("transfer ownership proposal");

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(NEW_OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .expect("accept ownership");

    let o = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(o.owner, Some(Addr::unchecked(NEW_OWNER)));
}

#[test]
fn update_ownership_transfer_rejected_for_non_owner() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(LENDER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ContractError::Ownership(cw_ownable::OwnershipError::NotOwner)
        ),
        "expected Ownership::NotOwner, got {:?}",
        err
    );
}

#[test]
fn update_ownership_rejected_with_funds() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn update_ownership_transfer_rejected_with_invalid_new_owner() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            // Invalid address: wrong bech32 prefix
            new_owner: "pb1q3xhmqrjukjuhmccy4p6xza6q0uxwclled4wrf".to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ContractError::Ownership(cw_ownable::OwnershipError::Std(_))
        ),
        "expected Ownership::Std, got {:?}",
        err
    );
}

#[test]
fn update_ownership_renounce_rejected() {
    let (mut deps, env) = setup();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::RenounceOwnership),
    )
    .unwrap_err();
    assert_eq!(
        err,
        illegal_argument("Renouncing contract ownership is not supported")
    );
}

#[test]
fn paused_allows_update_ownership() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    )
    .expect("set paused");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("transfer ownership when paused");

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(NEW_OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .expect("accept when paused");

    let o = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(o.owner, Some(Addr::unchecked(NEW_OWNER)));
}

#[test]
fn pending_ownership_overwritten_first_acceptor_rejected_second_accepts() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("first transfer proposal");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: SECOND_NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("second transfer proposal overwrites pending");

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(NEW_OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ContractError::Ownership(cw_ownable::OwnershipError::NotPendingOwner)
        ),
        "first pending owner should not accept after overwrite, got {:?}",
        err
    );

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(SECOND_NEW_OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .expect("second pending owner accepts");

    let o = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(o.owner, Some(Addr::unchecked(SECOND_NEW_OWNER)));
}

#[test]
fn pending_ownership_overwritten_by_original_owner() {
    let (mut deps, env) = setup();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("first transfer proposal");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: OWNER.to_string(),
            expiry: None,
        }),
    )
    .expect("second transfer proposal back to original owner overwrites pending");

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(NEW_OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ContractError::Ownership(cw_ownable::OwnershipError::NotPendingOwner)
        ),
        "first pending owner should not accept after overwrite, got {:?}",
        err
    );

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .expect("second pending owner accepts");

    let o = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(o.owner, Some(Addr::unchecked(OWNER)));
}
