//! Unit tests for pool_v2 utils/validation.rs: migration version, single-coin denom, lender/borrower attributes.

use crate::model::error::ContractError;
use crate::model::Denom;
use crate::utils::{validate_borrower_attrs, validate_lender_attrs, validate_single_coin_denom};
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{coin, Addr, Uint128};
use provwasm_mocks::mock_provenance_dependencies;
use provwasm_std::types::provenance::attribute::v1::{
    Attribute, AttributeType, QueryAttributeRequest, QueryAttributeResponse,
};

const LENDING_DENOM: &str = "uylds.fcc";

// ---- validate_single_coin_denom ----

fn lending_denom() -> Denom {
    Denom::new(LENDING_DENOM, 6u32)
}

#[test]
fn single_coin_denom_rejects_zero_coins() {
    let info = message_info(&Addr::unchecked("user"), &[]);
    let err = validate_single_coin_denom(&info, &lending_denom(), Uint128::new(1)).unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Exactly one coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn single_coin_denom_rejects_two_coins() {
    let info = message_info(
        &Addr::unchecked("user"),
        &[coin(100, LENDING_DENOM), coin(50, LENDING_DENOM)],
    );
    let err = validate_single_coin_denom(&info, &lending_denom(), Uint128::new(1)).unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Exactly one coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn single_coin_denom_rejects_wrong_denom() {
    let info = message_info(&Addr::unchecked("user"), &[coin(100, "other.denom")]);
    let err = validate_single_coin_denom(&info, &lending_denom(), Uint128::new(1)).unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Expected denom"));
            assert!(message.contains(LENDING_DENOM));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn single_coin_denom_rejects_below_minimum() {
    let info = message_info(&Addr::unchecked("user"), &[coin(50, LENDING_DENOM)]);
    let min = Uint128::new(100);
    let err = validate_single_coin_denom(&info, &lending_denom(), min).unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("below minimum"));
            assert!(message.contains("50"));
            assert!(message.contains("100"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn single_coin_denom_ok_when_equal_to_minimum() {
    let info = message_info(&Addr::unchecked("user"), &[coin(100, LENDING_DENOM)]);
    let amount = validate_single_coin_denom(&info, &lending_denom(), Uint128::new(100)).unwrap();
    assert_eq!(amount, Uint128::new(100));
}

#[test]
fn single_coin_denom_ok_when_above_minimum() {
    let info = message_info(&Addr::unchecked("user"), &[coin(1_000_000, LENDING_DENOM)]);
    let amount = validate_single_coin_denom(&info, &lending_denom(), Uint128::new(1)).unwrap();
    assert_eq!(amount, Uint128::new(1_000_000));
}

// ---- validate_lender_attrs / validate_borrower_attrs (require mocked attribute querier) ----

fn setup_empty_attributes(querier: &mut provwasm_mocks::MockProvenanceQuerier, account: &str) {
    let response = QueryAttributeResponse {
        account: account.to_string(),
        attributes: vec![],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(querier, response);
}

fn setup_attribute(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    account: &str,
    name: &str,
    value: &str,
) {
    let response = QueryAttributeResponse {
        account: account.to_string(),
        attributes: vec![Attribute {
            name: name.to_string(),
            value: value.as_bytes().to_vec(),
            attribute_type: AttributeType::String.into(),
            address: "".to_string(),
            expiration_date: None,
            concrete_type: "".to_owned(),
        }],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(querier, response);
}

#[test]
fn lender_attrs_ok_when_empty_list() {
    let deps = mock_provenance_dependencies();
    validate_lender_attrs(&deps.as_ref().querier, "tp1user", &[]).unwrap();
}

#[test]
fn lender_attrs_fail_when_none_match() {
    let mut deps = mock_provenance_dependencies();
    setup_empty_attributes(&mut deps.querier, "tp1user");
    let err = validate_lender_attrs(
        &deps.as_ref().querier,
        "tp1user",
        &["lender.kyc".to_string()],
    )
    .unwrap_err();
    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("lender"));
            assert!(message.contains("lender.kyc"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn lender_attrs_ok_when_one_present() {
    let mut deps = mock_provenance_dependencies();
    setup_attribute(&mut deps.querier, "tp1user", "lender.kyc", "verified");
    validate_lender_attrs(
        &deps.as_ref().querier,
        "tp1user",
        &["lender.kyc".to_string()],
    )
    .unwrap();
}

/// With "all required": having all listed attrs must pass.
#[test]
fn lender_attrs_ok_when_all_present() {
    let mut deps = mock_provenance_dependencies();
    setup_attribute(&mut deps.querier, "tp1user", "lender.kyc", "verified");
    setup_attribute(&mut deps.querier, "tp1user", "lender.accredited", "true");
    validate_lender_attrs(
        &deps.as_ref().querier,
        "tp1user",
        &["lender.kyc".to_string(), "lender.accredited".to_string()],
    )
    .unwrap();
}

#[test]
fn borrower_attrs_ok_when_empty_list() {
    let deps = mock_provenance_dependencies();
    validate_borrower_attrs(&deps.as_ref().querier, "tp1borrower", &[]).unwrap();
}

#[test]
fn borrower_attrs_fail_when_none_match() {
    let mut deps = mock_provenance_dependencies();
    setup_empty_attributes(&mut deps.querier, "tp1borrower");
    let err = validate_borrower_attrs(
        &deps.as_ref().querier,
        "tp1borrower",
        &["borrower.kyc".to_string()],
    )
    .unwrap_err();
    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("borrower"));
            assert!(message.contains("borrower.kyc"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn borrower_attrs_ok_when_one_present() {
    let mut deps = mock_provenance_dependencies();
    setup_attribute(&mut deps.querier, "tp1borrower", "borrower.kyc", "verified");
    validate_borrower_attrs(
        &deps.as_ref().querier,
        "tp1borrower",
        &["borrower.kyc".to_string()],
    )
    .unwrap();
}
