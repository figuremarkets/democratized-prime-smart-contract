//! Tests for GetBorrowerStatus query (require_commit).

use crate::contract::{execute, query};
use crate::model::query::BorrowerStatusResponseV1;
use crate::msg::{ExecuteMsg, QueryMsg};
use crate::tests::query::common::{setup_instantiated, CUSTODIAN, OWNER, SOME_USER};
use cosmwasm_std::from_json;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::Addr;

const BORROWER: &str = SOME_USER;

#[test]
fn get_borrower_status_default_is_false() {
    let (deps, env) = setup_instantiated();
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerStatus {
            address: BORROWER.to_string(),
        },
    )
    .expect("query should succeed");
    let res: BorrowerStatusResponseV1 = from_json(bin).expect("decode GetBorrowerStatus response");
    assert!(!res.require_commit);
}

#[test]
fn get_borrower_status_after_set_require_commit_true() {
    let (mut deps, env) = setup_instantiated();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            max_liquidation_staleness_seconds: None,
            liquidation_access: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
            custodian: None,
            liquidator: None,
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .expect("set require commit should succeed");
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerStatus {
            address: BORROWER.to_string(),
        },
    )
    .expect("query should succeed");
    let res: BorrowerStatusResponseV1 = from_json(bin).expect("decode GetBorrowerStatus response");
    assert!(res.require_commit);
}

#[test]
fn get_borrower_status_false_after_disable_and_clear() {
    let (mut deps, env) = setup_instantiated();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(CUSTODIAN), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            max_liquidation_staleness_seconds: None,
            liquidation_access: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
            custodian: None,
            liquidator: None,
        },
    )
    .expect("set commit_market_id");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .expect("enable");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(false),
        },
    )
    .expect("disable");
    let bin = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::GetBorrowerStatus {
            address: BORROWER.to_string(),
        },
    )
    .expect("query");
    let res: BorrowerStatusResponseV1 = from_json(bin).unwrap();
    assert!(!res.require_commit);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: Some(true),
        },
    )
    .expect("enable again");
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::SetBorrowerRequireCommit {
            address: BORROWER.to_string(),
            require: None,
        },
    )
    .expect("clear");
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerStatus {
            address: BORROWER.to_string(),
        },
    )
    .expect("query");
    let res: BorrowerStatusResponseV1 = from_json(bin).unwrap();
    assert!(!res.require_commit);
}

#[test]
fn get_borrower_status_invalid_address_fails() {
    let (deps, env) = setup_instantiated();
    let err = query(
        deps.as_ref(),
        env,
        QueryMsg::GetBorrowerStatus {
            address: "not-an-address".to_string(),
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("invalid")
            || msg.to_lowercase().contains("address")
            || msg.to_lowercase().contains("bech32")
    );
}
