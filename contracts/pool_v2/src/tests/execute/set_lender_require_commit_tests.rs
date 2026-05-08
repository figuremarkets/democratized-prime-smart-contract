//! Tests for SetLenderRequireCommitOnExit (contract owner only). No funds accepted.

use crate::contract::{execute, query};
use crate::execute::set_lender_require_commit::ASSERT_OWNER_ERR;
use crate::model::query::LenderStatusResponseV1;
use crate::msg::{ExecuteMsg, QueryMsg};
use crate::tests::instantiate_helpers::{setup_instantiated_contract, LENDING_DENOM, OWNER};
use cosmwasm_std::from_json;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{coin, Addr};
use democratized_prime_lib::common::ContractError;

/// Valid Provenance bech32 for addr_validate in SetLenderRequireCommitOnExit.
const LENDER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";

#[test]
fn set_lender_require_commit_fails_when_no_commit_market_id() {
    let (mut deps, env) = setup_instantiated_contract();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("Commit market must be configured"),
                "expected commit_market_id message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn set_lender_require_commit_owner_succeeds_and_persists() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("owner set_lender_require_commit should succeed");

    let bin = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::GetLenderStatus {
            address: LENDER.to_string(),
        },
    )
    .expect("query");
    let res: LenderStatusResponseV1 = from_json(bin).unwrap();
    assert!(res.require_commit_on_exit);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(false),
        },
    )
    .expect("clear require should succeed");

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetLenderStatus {
            address: LENDER.to_string(),
        },
    )
    .expect("query");
    let res: LenderStatusResponseV1 = from_json(bin).unwrap();
    assert!(!res.require_commit_on_exit);
}

#[test]
fn set_lender_require_commit_non_owner_fails() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(LENDER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn set_lender_require_commit_fails_with_funds() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn set_lender_require_commit_none_removes_override() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("set true");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetLenderRequireCommitOnExit {
            address: LENDER.to_string(),
            require: None,
        },
    )
    .expect("remove override");

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetLenderStatus {
            address: LENDER.to_string(),
        },
    )
    .expect("query");
    let res: LenderStatusResponseV1 = from_json(bin).unwrap();
    assert!(!res.require_commit_on_exit);
}
