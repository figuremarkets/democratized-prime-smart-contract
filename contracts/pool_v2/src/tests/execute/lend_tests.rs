//! Tests for the Supply execute: success with valid funds and optional lender attr,
//! failures for no funds, wrong denom, below `min_lend`, and missing required lender attribute.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_SCALED_AMOUNT};
use crate::contract::execute;
use crate::execute::lend::ACTION;
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::ReserveStateV1;
use crate::msg::ExecuteMsg;
use crate::storage::{get_reserve_state_v1, set_reserve_state_v1};
use crate::tests::instantiate_helpers::{
    default_instantiate_msg, setup_instantiated_contract, LENDER, LENDING_DENOM, OWNER,
    REPO_TOKEN_CW20,
};
use crate::tests::reserve_invariant::assert_reserve_assets_liabilities_tie_out_with_tolerance;
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{coin, Addr, CosmosMsg, Decimal256, Uint128, WasmMsg};
use provwasm_mocks::mock_provenance_dependencies;
use provwasm_std::types::provenance::attribute::v1::{
    Attribute, AttributeType, QueryAttributeRequest, QueryAttributeResponse,
};

fn mock_empty_attribute_response(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    account: &str,
) {
    let response = QueryAttributeResponse {
        account: account.to_string(),
        attributes: vec![],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(querier, response);
}
use std::str::FromStr;

#[test]
fn lend_succeeds_with_valid_funds_and_updates_reserve() {
    let (mut deps, env) = setup_instantiated_contract();
    let amount = Uint128::new(100_000_000);
    let info = message_info(
        &Addr::unchecked(LENDER),
        &[coin(amount.u128(), LENDING_DENOM)],
    );

    let res = execute(deps.as_mut(), env.clone(), info, ExecuteMsg::Lend {})
        .expect("lend should succeed");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { contract_addr, .. }) => {
            assert_eq!(
                contract_addr.as_str(),
                REPO_TOKEN_CW20,
                "expected CW20 mint to repo token contract"
            );
        }
        _ => panic!("expected Wasm Execute (CW20 mint) message"),
    }
    assert_eq!(res.attributes.len(), 9);
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, "lender");
    assert_eq!(res.attributes[1].value, LENDER);
    assert_eq!(res.attributes[2].key, "amount");
    assert_eq!(res.attributes[2].value, amount.to_string());

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(
        res.attributes[3].value,
        reserve.total_scaled_liquidity.to_string()
    );
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
    assert!(reserve.total_scaled_liquidity > 0);
    // Lend uses floor(amount/index); at liquidity_index 1.0 the accounting matches flows (tolerance 0).
    const TOLERANCE: u128 = 0;
    assert_reserve_assets_liabilities_tie_out_with_tolerance(
        deps.as_ref().storage,
        "after lend",
        Some(amount.u128()),
        TOLERANCE,
    )
    .unwrap();
}

#[test]
fn lend_fails_with_no_funds() {
    let (mut deps, env) = setup_instantiated_contract();
    let info = message_info(&Addr::unchecked(LENDER), &[]);

    let err = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Exactly one coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn lend_fails_with_wrong_denom() {
    let (mut deps, env) = setup_instantiated_contract();
    let info = message_info(
        &Addr::unchecked(LENDER),
        &[coin(100_000_000, "wrong.denom")],
    );

    let err = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Expected denom"));
            assert!(message.contains(LENDING_DENOM));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn lend_fails_when_below_min_lend() {
    let (mut deps, env) = setup_instantiated_contract();
    let info = message_info(&Addr::unchecked(LENDER), &[coin(0, LENDING_DENOM)]);

    let err = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("below minimum"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

/// With floor-on-lend, `underlying_to_scaled_liquidity(amount, index)` can be 0 when `amount` is
/// tiny relative to `liquidity_index` (e.g. 1 base unit at index 2.0). Minting zero repo shares
/// must be rejected so the vault does not accept funds without recording supply.
#[test]
fn lend_fails_when_amount_floors_to_zero_scaled_liquidity() {
    let (mut deps, env) = setup_instantiated_contract();

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let bumped = ReserveStateV1 {
        liquidity_index: Decimal256::from_str("2").unwrap(),
        borrow_index: reserve.borrow_index,
        last_updated_at: env.block.time,
        total_scaled_liquidity: 1_000_000,
        total_scaled_borrow: 0,
        accrued_reserve: reserve.accrued_reserve,
        deficit_underlying: reserve.deficit_underlying,
    };
    set_reserve_state_v1(deps.as_mut().storage, &bumped).unwrap();

    let info = message_info(&Addr::unchecked(LENDER), &[coin(1, LENDING_DENOM)]);
    let err = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("rounds to zero"),
                "unexpected message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn lend_fails_when_lender_attr_required_but_missing() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    let mut msg = default_instantiate_msg();
    msg.lender_required_attrs = vec!["lender.kyc".to_string()];
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");

    mock_empty_attribute_response(&mut deps.querier, LENDER);

    let info = message_info(
        &Addr::unchecked(LENDER),
        &[coin(100_000_000, LENDING_DENOM)],
    );
    let err = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).unwrap_err();

    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("lender"));
            assert!(message.contains("lender.kyc"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn lend_succeeds_when_lender_attr_required_and_present() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();

    let attr_response = QueryAttributeResponse {
        account: LENDER.to_string(),
        attributes: vec![Attribute {
            name: "lender.kyc".to_string(),
            value: b"verified".to_vec(),
            attribute_type: AttributeType::String.into(),
            address: "".to_string(),
            expiration_date: None,
            concrete_type: "".to_owned(),
        }],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(&mut deps.querier, attr_response);

    let mut msg = default_instantiate_msg();
    msg.lender_required_attrs = vec!["lender.kyc".to_string()];
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");

    let amount = Uint128::new(50_000_000);
    let info = message_info(
        &Addr::unchecked(LENDER),
        &[coin(amount.u128(), LENDING_DENOM)],
    );
    let res = execute(deps.as_mut(), env, info, ExecuteMsg::Lend {}).expect("lend should succeed");

    assert_eq!(res.messages.len(), 1);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].value, LENDER);
    assert_eq!(res.attributes[2].value, amount.to_string());
    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(res.attributes[3].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(
        res.attributes[3].value,
        reserve.total_scaled_liquidity.to_string()
    );
    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);
}
