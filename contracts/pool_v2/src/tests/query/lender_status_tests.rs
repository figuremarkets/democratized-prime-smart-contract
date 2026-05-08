//! Tests for GetLenderStatus query (require_commit_on_exit).

use crate::contract::{execute, query};
use crate::model::query::LenderStatusResponseV1;
use crate::msg::{ExecuteMsg, QueryMsg};
use crate::tests::query::common::{setup_instantiated, OWNER, SOME_USER};
use cosmwasm_std::from_json;
use cosmwasm_std::testing::message_info;
use cosmwasm_std::Addr;

const LENDER: &str = SOME_USER;

#[test]
fn get_lender_status_default_is_false() {
    let (deps, env) = setup_instantiated();
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetLenderStatus {
            address: LENDER.to_string(),
        },
    )
    .expect("query should succeed");
    let res: LenderStatusResponseV1 = from_json(bin).expect("decode GetLenderStatus response");
    assert!(!res.require_commit_on_exit);
}

#[test]
fn get_lender_status_after_set_require_commit_true() {
    let (mut deps, env) = setup_instantiated();
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
    .expect("set require commit should succeed");
    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::GetLenderStatus {
            address: LENDER.to_string(),
        },
    )
    .expect("query should succeed");
    let res: LenderStatusResponseV1 = from_json(bin).expect("decode GetLenderStatus response");
    assert!(res.require_commit_on_exit);
}

#[test]
fn get_lender_status_invalid_address_fails() {
    let (deps, env) = setup_instantiated();
    let err = query(
        deps.as_ref(),
        env,
        QueryMsg::GetLenderStatus {
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
