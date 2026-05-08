//! Tests for Transfer execute: success (exact and with refund); failures for zero amount,
//! sender equals recipient, invalid recipient, insufficient repo token sent, recipient missing lender attr.
//! Also tests for TransferExact: success, zero amount, no funds, two coins, wrong denom, sender eq recipient, invalid recipient, recipient missing lender attr.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_LENDER, ATTRIBUTE_RECIPIENT,
    ATTRIBUTE_SCALED_AMOUNT,
};
use crate::contract::execute;
use crate::execute::transfer::{ACTION, ACTION_EXACT};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::execute::Cw20ReceivePayload;
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_effective_reserve;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    to_json_binary, Addr, CosmosMsg, Decimal256, Env, MemoryStorage, OwnedDeps, Uint128, WasmMsg,
};
use cw20::Cw20ReceiveMsg;
use provwasm_mocks::mock_provenance_dependencies;
use provwasm_std::types::provenance::attribute::v1::{
    QueryAttributeRequest, QueryAttributeResponse,
};
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const SENDER: &str = "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu";
const RECIPIENT: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";
const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-demo".to_string(),
        description: "Test pool v2".to_string(),
        repo_token: RepoTokenConfig::Existing {
            repo_token_cw20_contract_address: REPO_TOKEN_CW20.to_string(),
        },
        lending_denom: Denom::new(LENDING_DENOM, 6u32),
        rate_params: RateParamsV1 {
            target_rate: Decimal256::from_str("0.09").unwrap(),
            min_rate: Decimal256::from_str("0.0325").unwrap(),
            max_rate: Decimal256::from_str("0.20").unwrap(),
            kink_utilization: Decimal256::from_str("0.90").unwrap(),
            reserve_factor: Decimal256::from_str("0.005").unwrap(),
            fee_model: Default::default(),
            flat_fee_apr: Decimal256::zero(),
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![CollateralAssetV1 {
            asset_id: "asset.one".to_string(),
            haircut: Some(Decimal256::percent(80)),
        }],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
    }
}

fn setup_instantiated() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let msg = default_instantiate_msg();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");
    (deps, env)
}

/// At instantiate, liquidity_index = 1, so scaled equals underlying.
#[test]
fn transfer_succeeds() {
    let (mut deps, env) = setup_instantiated();
    let block_time = env.block.time;
    let amount_underlying = 5_000_000u128;
    let scaled = amount_underlying; // index = 1 at instantiate
    let amount = Uint128::new(amount_underlying);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount,
            })
            .unwrap(),
        }),
    )
    .expect("transfer should succeed");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { contract_addr, .. }) => {
            assert_eq!(contract_addr.as_str(), REPO_TOKEN_CW20);
        }
        _ => panic!("expected Wasm Execute (CW20 Transfer) to recipient"),
    }
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, ATTRIBUTE_LENDER);
    assert_eq!(res.attributes[1].value, SENDER);
    assert_eq!(res.attributes[2].key, ATTRIBUTE_RECIPIENT);
    assert_eq!(res.attributes[2].value, RECIPIENT);
    assert_eq!(res.attributes[3].key, ATTRIBUTE_AMOUNT);
    assert_eq!(res.attributes[3].value, amount.to_string());
    assert_eq!(res.attributes[4].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[4].value, scaled.to_string());
    assert_response_lend_borrow_rates_match_effective_reserve(
        &res,
        deps.as_ref().storage,
        block_time,
    );
}

#[test]
fn transfer_succeeds_with_refund() {
    let (mut deps, env) = setup_instantiated();
    let block_time = env.block.time;
    let amount_underlying = 3_000_000u128;
    let scaled_needed = amount_underlying;
    let sent_scaled = scaled_needed + 2_000_000u128;
    let amount = Uint128::new(amount_underlying);

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(sent_scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount,
            })
            .unwrap(),
        }),
    )
    .expect("transfer should succeed");

    assert_eq!(
        res.messages.len(),
        2,
        "Transfer to recipient + refund to sender"
    );
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { contract_addr, .. }) => {
            assert_eq!(contract_addr.as_str(), REPO_TOKEN_CW20);
        }
        _ => panic!("first message should be CW20 Transfer to recipient"),
    }
    match &res.messages[1].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { .. }) => {}
        _ => panic!("second message should be CW20 Transfer refund to sender"),
    }
    assert_response_lend_borrow_rates_match_effective_reserve(
        &res,
        deps.as_ref().storage,
        block_time,
    );
}

#[test]
fn transfer_fails_zero_amount() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount: Uint128::zero(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Transfer amount must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_fails_sender_equals_recipient() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: SENDER.to_string(),
                amount: Uint128::new(1000),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Recipient must be different from sender"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_fails_invalid_recipient() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: "not-a-valid-address!!!".to_string(),
                amount: Uint128::new(1000),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::Std(_) => {}
        _ => panic!("expected StdError (invalid address), got {:?}", err),
    }
}

#[test]
fn transfer_fails_insufficient_repo_token_sent() {
    let (mut deps, env) = setup_instantiated();
    let amount_underlying = 10_000_000u128;
    let sent_scaled = 1_000_000u128; // less than amount_underlying when index = 1

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(sent_scaled),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount: Uint128::new(amount_underlying),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Insufficient repo token sent"));
            assert!(message.contains("10000000"));
            assert!(message.contains("1000000"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

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

#[test]
fn transfer_fails_wrong_cw20_sender() {
    let (mut deps, env) = setup_instantiated();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("other_contract"), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount: Uint128::new(1000),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Only the repo token CW20"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_fails_recipient_missing_lender_attr() {
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

    mock_empty_attribute_response(&mut deps.querier, RECIPIENT);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(5_000_000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount: Uint128::new(5_000_000),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("lender"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

// --- TransferExact tests ---

#[test]
fn transfer_exact_succeeds_forwards_sent_repo_to_recipient() {
    let (mut deps, env) = setup_instantiated();
    let block_time = env.block.time;
    let amount = 7_000_000u128;

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(amount),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: RECIPIENT.to_string(),
            })
            .unwrap(),
        }),
    )
    .expect("transfer_exact should succeed");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute { contract_addr, .. }) => {
            assert_eq!(contract_addr.as_str(), REPO_TOKEN_CW20);
        }
        _ => panic!("expected Wasm Execute (CW20 Transfer) to recipient"),
    }
    assert_eq!(res.attributes[0].value, ACTION_EXACT);
    assert_eq!(res.attributes[3].value, amount.to_string());
    assert_eq!(res.attributes[4].key, ATTRIBUTE_SCALED_AMOUNT);
    assert_eq!(res.attributes[4].value, amount.to_string());
    assert_response_lend_borrow_rates_match_effective_reserve(
        &res,
        deps.as_ref().storage,
        block_time,
    );
}

#[test]
fn transfer_exact_fails_zero_amount() {
    let (mut deps, env) = setup_instantiated();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::zero(),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: RECIPIENT.to_string(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Transfer amount must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_exact_fails_wrong_cw20_sender() {
    let (mut deps, env) = setup_instantiated();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("other_contract"), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: RECIPIENT.to_string(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Only the repo token CW20"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_exact_fails_sender_equals_recipient() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: SENDER.to_string(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Recipient must be different from sender"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn transfer_exact_fails_invalid_recipient() {
    let (mut deps, env) = setup_instantiated();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1000u128),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: "not-a-valid-address!!!".to_string(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::Std(_) => {}
        _ => panic!("expected StdError (invalid address), got {:?}", err),
    }
}

#[test]
fn transfer_exact_fails_recipient_missing_lender_attr() {
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

    mock_empty_attribute_response(&mut deps.querier, RECIPIENT);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(5_000_000u128),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: RECIPIENT.to_string(),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("lender"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn transfer_fails_when_require_commit_on_exit() {
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
            address: SENDER.to_string(),
            require: Some(true),
        },
    )
    .expect("set require commit should succeed");

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(REPO_TOKEN_CW20), &[]),
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: SENDER.to_string(),
            amount: Uint128::from(1_000_000u128),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: RECIPIENT.to_string(),
                amount: Uint128::new(1_000_000),
            })
            .unwrap(),
        }),
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("Transfers are not allowed while commitment-on-exit is required")
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
