//! Helpers to simulate repo token `SubMsg` + `reply` in unit tests (mock chain does not execute submessages).

use crate::constants::REPO_TOKEN_INSTANTIATE_REPLY_ID;
use crate::instantiate::instantiate_contract;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{InstantiateMsg, RepoTokenConfig};
use cosmwasm_std::testing::message_info;
use cosmwasm_std::{
    testing::{mock_env, MockApi},
    Binary, Env, MemoryStorage, MsgResponse, OwnedDeps, Reply, SubMsgResponse, SubMsgResult,
};
use cosmwasm_std::{Addr, Decimal256, Uint128};
use provwasm_mocks::mock_provenance_dependencies;
use std::str::FromStr;

/// Protobuf `Any` type URL for `MsgInstantiateContractResponse` on wasmd (mirrors real `msg_responses` entries).
const MSG_INSTANTIATE_CONTRACT_RESPONSE_TYPE_URL: &str =
    "/cosmwasm.wasm.v1.MsgInstantiateContractResponse";

/// Minimal protobuf `MsgInstantiateContractResponse` with `contract_address` (field 1) only.
pub fn encode_instantiate_reply_contract_address(addr: &str) -> Binary {
    let bytes = addr.as_bytes();
    let mut v = Vec::with_capacity(2 + bytes.len());
    v.push(0x0a);
    v.push(bytes.len() as u8);
    v.extend_from_slice(bytes);
    Binary::new(v)
}

/// Builds a successful `Reply` like wasmd on CosmWasm 2+ (`SubMsgResponse.msg_responses`).
///
/// `SubMsgResponse.data` is still set to `None` only because the struct requires the field; it is
/// not read by pool `reply` handling.
#[allow(deprecated)]
pub fn mock_repo_token_instantiate_reply(repo_token_contract: &str) -> Reply {
    let value = encode_instantiate_reply_contract_address(repo_token_contract);
    Reply {
        id: REPO_TOKEN_INSTANTIATE_REPLY_ID,
        payload: Default::default(),
        gas_used: 0,
        result: SubMsgResult::Ok(SubMsgResponse {
            events: vec![],
            data: None,
            msg_responses: vec![MsgResponse {
                type_url: MSG_INSTANTIATE_CONTRACT_RESPONSE_TYPE_URL.to_string(),
                value,
            }],
        }),
    }
}

pub const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
pub const LENDER: &str = "tp1lender";
/// "u" prefix => 1 ylds.fcc = 10^6 uylds.fcc.
pub const LENDING_DENOM: &str = "uylds.fcc";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
pub const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
pub const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";

pub fn default_instantiate_msg() -> InstantiateMsg {
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

pub fn setup_instantiated_contract() -> (
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
