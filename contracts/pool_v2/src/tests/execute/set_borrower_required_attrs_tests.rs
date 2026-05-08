//! Tests for SetBorrowRequiredAttrs (contract owner only). Borrower validation requires all attributes in the list.
//! No funds accepted.

use crate::constants::MAX_LENDER_BORROWER_REQUIRED_ATTRS;
use crate::contract::execute;
use crate::execute::set_borrower_required_attrs::ASSERT_OWNER_ERR;
use crate::model::error::ContractError;
use crate::msg::ExecuteMsg;
use crate::storage::get_contract_state_v1;
use crate::tests::instantiate_helpers::{
    setup_instantiated_contract, LENDER, LENDING_DENOM, OWNER,
};
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{coin, Addr};

#[test]
fn set_borrower_required_attrs_owner_succeeds_and_updates_state() {
    let (mut deps, env) = setup_instantiated_contract();
    let attrs = vec![
        "borrower.kyc".to_string(),
        "borrower.accredited".to_string(),
    ];
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: attrs.clone(),
        },
    )
    .expect("owner set_borrower_required_attrs should succeed");
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.borrower_required_attrs, attrs);
}

#[test]
fn set_borrower_required_attrs_non_owner_fails() {
    let (mut deps, env) = setup_instantiated_contract();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(LENDER), &[]),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: vec!["borrower.kyc".to_string()],
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn set_borrower_required_attrs_fails_with_funds() {
    let (mut deps, env) = setup_instantiated_contract();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, LENDING_DENOM)]),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: vec![],
        },
    )
    .unwrap_err();
    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn set_borrower_required_attrs_empty_list_allowed() {
    let (mut deps, env) = setup_instantiated_contract();
    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: vec![],
        },
    )
    .expect("set empty list should succeed");
    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert!(contract.borrower_required_attrs.is_empty());
}

#[test]
fn set_borrower_required_attrs_fails_when_over_max_attrs() {
    let (mut deps, env) = setup_instantiated_contract();
    let attrs: Vec<String> = (0..MAX_LENDER_BORROWER_REQUIRED_ATTRS + 1)
        .map(|i| format!("borrower.attr.{i}"))
        .collect();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: attrs,
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains(&MAX_LENDER_BORROWER_REQUIRED_ATTRS.to_string())
                    && message.to_lowercase().contains("borrower"),
                "expected max borrower attrs message, got {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
