//! Tests for SetBorrowerRequireCommit (contract owner only). No funds accepted.

use crate::contract::{execute, query};
use crate::execute::set_borrower_require_commit::ASSERT_OWNER_ERR;
use crate::model::query::BorrowerStatusResponseV1;
use crate::msg::{ExecuteMsg, QueryMsg};
use crate::tests::execute::set_commit_market_id;
use crate::tests::instantiate_helpers::{setup_instantiated_contract, LENDING_DENOM};
use crate::tests::query::common::{CUSTODIAN, OWNER};
use cosmwasm_std::from_json;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{coin, Addr};
use democratized_prime_lib::common::ContractError;

/// Valid Provenance bech32 for addr_validate in SetBorrowerRequireCommit.
const BORROWER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";

fn borrower_status(
    deps: &cosmwasm_std::OwnedDeps<
        cosmwasm_std::MemoryStorage,
        cosmwasm_std::testing::MockApi,
        provwasm_mocks::MockProvenanceQuerier,
    >,
    env: &cosmwasm_std::Env,
) -> BorrowerStatusResponseV1 {
    let bin = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::GetBorrowerStatus {
            address: BORROWER.to_string(),
        },
    )
    .expect("query");
    from_json(bin).unwrap()
}

#[test]
fn set_borrower_require_commit_fails_when_no_commit_market_id() {
    let (mut deps, env) = setup_instantiated_contract();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
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
fn set_borrower_require_commit_owner_enables_and_disables() {
    let (mut deps, env) = setup_instantiated_contract();
    set_commit_market_id(&mut deps, &env);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .expect("owner set_borrower_require_commit should succeed");
    assert!(borrower_status(&deps, &env).require_commit);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(false),
        },
    )
    .expect("disable require should succeed");
    assert!(!borrower_status(&deps, &env).require_commit);
}

#[test]
fn set_borrower_require_commit_none_clears() {
    let (mut deps, env) = setup_instantiated_contract();
    set_commit_market_id(&mut deps, &env);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .expect("set true");

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: None,
        },
    )
    .expect("clear require");
    assert!(!borrower_status(&deps, &env).require_commit);
}

#[test]
fn set_borrower_require_commit_custodian_fails() {
    let (mut deps, env) = setup_instantiated_contract();
    set_commit_market_id(&mut deps, &env);
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
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
fn set_borrower_require_commit_non_owner_fails() {
    let (mut deps, env) = setup_instantiated_contract();
    set_commit_market_id(&mut deps, &env);
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(BORROWER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
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
fn set_borrower_require_commit_fails_with_funds() {
    let (mut deps, env) = setup_instantiated_contract();
    set_commit_market_id(&mut deps, &env);
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}
