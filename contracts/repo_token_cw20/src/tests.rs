//! Unit tests for repo_token_cw20.
//!
//! We focus on **custom behavior**: Balance/TokenInfo returning underlying when pool_address is set,
//! auth (minter-only mint/burn, owner-only UpdateConfig), and Send/Transfer rules (Send → pool only;
//! Transfer pool-only as sender). We do not duplicate full tests for standard CW20 balance math.

use crate::constants::{DEFAULT_ALL_ACCOUNTS_PAGE_SIZE, MAX_ALL_ACCOUNTS_PAGE_SIZE};
use crate::contract::{execute, instantiate, query};
use crate::error::ContractError;
use crate::execute::UPDATE_CONFIG_ASSERT_OWNER_ERR;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::pool_query::{PoolReserveResponse, PoolReserveState};
use crate::state::BALANCES;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{
    coin, from_json, to_json_binary, Addr, ContractResult, MemoryStorage, OwnedDeps, QuerierResult,
    SystemError, SystemResult, Uint128, WasmQuery,
};
use cw20::{AllAccountsResponse, BalanceResponse, TokenInfoResponse};
use cw_ownable::{get_ownership, Action};
use provwasm_mocks::mock_provenance_dependencies;

// Valid Provenance bech32 (tp1) addresses so addr_validate passes in tests.
const ADMIN: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const MINTER: &str = "tp1wvefn22cq723u98f6mdqykf6w6avfckjzp6rtz";
const POOL: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const USER: &str = "tp1w9p4tkctug2jyyx663f77x7e5cdry067z6xee4";
const NEW_ADMIN: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        name: "Test Receipt".to_string(),
        symbol: "TRC".to_string(), // 3+ chars to pass cw20-base-style validation
        decimals: 6,
        owner: ADMIN.to_string(),
        minter: MINTER.to_string(),
        pool_address: None,
    }
}

fn fetch_all_accounts(
    deps: &OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    start_after: Option<String>,
    limit: Option<u32>,
) -> AllAccountsResponse {
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::AllAccounts { start_after, limit },
    )
    .unwrap();
    from_json(bin).unwrap()
}

fn instantiate_default(
    deps: &mut OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
) {
    let env = mock_env();
    instantiate(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(ADMIN), &[]),
        default_instantiate_msg(),
    )
    .unwrap();
}

/// Mock the pool's GetReserve to return the given liquidity_index (e.g. "1.05").
fn mock_pool_reserve(
    querier: &mut provwasm_mocks::MockProvenanceQuerier,
    pool_addr: &str,
    liquidity_index: &str,
) {
    let pool_addr = pool_addr.to_string();
    let liquidity_index = liquidity_index.to_string();
    let handler = move |query: &WasmQuery| -> QuerierResult {
        match query {
            WasmQuery::Smart {
                contract_addr,
                msg: _,
            } if contract_addr.as_str() == pool_addr => SystemResult::Ok(ContractResult::Ok(
                to_json_binary(&PoolReserveResponse {
                    reserve: PoolReserveState {
                        liquidity_index: liquidity_index.clone(),
                    },
                })
                .unwrap(),
            )),
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "expected pool GetReserve".to_string(),
            }),
        }
    };
    querier.mock_querier.update_wasm(handler);
}

// --- Query: Balance returns scaled when no pool ---

#[test]
fn query_balance_without_pool_returns_scaled() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    // Mint some scaled balance (as minter)
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1_000_000u128),
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::Balance {
            address: USER.to_string(),
        },
    )
    .unwrap();
    let res: BalanceResponse = from_json(bin).unwrap();
    assert_eq!(res.balance, Uint128::from(1_000_000u128));
}

// --- Query: Balance returns underlying when pool is set ---

#[test]
fn query_balance_with_pool_returns_underlying() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = InstantiateMsg {
        pool_address: Some(POOL.to_string()),
        ..default_instantiate_msg()
    };
    let env = mock_env();
    instantiate(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1_000_000u128),
        },
    )
    .unwrap();
    mock_pool_reserve(&mut deps.querier, POOL, "1.05");

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::Balance {
            address: USER.to_string(),
        },
    )
    .unwrap();
    let res: BalanceResponse = from_json(bin).unwrap();
    // 1_000_000 * 1.05 = 1_050_000 (floor)
    assert_eq!(res.balance, Uint128::from(1_050_000u128));
}

// --- Query: TokenInfo total_supply is scaled without pool, underlying with pool ---

#[test]
fn query_token_info_total_supply_without_pool_is_scaled() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(500_000u128),
        },
    )
    .unwrap();

    let bin = query(deps.as_ref(), env, QueryMsg::TokenInfo {}).unwrap();
    let info: TokenInfoResponse = from_json(bin).unwrap();
    assert_eq!(info.total_supply, Uint128::from(500_000u128));
}

#[test]
fn query_token_info_total_supply_with_pool_is_underlying() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = InstantiateMsg {
        pool_address: Some(POOL.to_string()),
        ..default_instantiate_msg()
    };
    let env = mock_env();
    instantiate(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(200_000u128),
        },
    )
    .unwrap();
    mock_pool_reserve(&mut deps.querier, POOL, "1.1");

    let bin = query(deps.as_ref(), env, QueryMsg::TokenInfo {}).unwrap();
    let info: TokenInfoResponse = from_json(bin).unwrap();
    // 200_000 * 1.1 = 220_000
    assert_eq!(info.total_supply, Uint128::from(220_000u128));
}

// --- Auth: only minter can mint ---

#[test]
fn mint_rejected_when_not_minter() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(100u128),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NotAuthorizedError { .. }));
}

// --- Query: ScaledBalance returns raw stored balance ---

#[test]
fn query_scaled_balance_returns_stored_balance() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(2_500_000u128),
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::ScaledBalance {
            address: USER.to_string(),
        },
    )
    .unwrap();
    let res: BalanceResponse = from_json(bin).unwrap();
    assert_eq!(res.balance, Uint128::from(2_500_000u128));
}

// --- Auth: only minter can burn ---

#[test]
fn burn_rejected_when_not_minter() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(100u128),
        },
    )
    .unwrap();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Burn {
            amount: Uint128::from(50u128),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NotAuthorizedError { .. }));
}

// --- BurnFrom: only minter can burn from another address ---

#[test]
fn burn_from_succeeds_when_minter() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1_000_000u128),
        },
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::BurnFrom {
            owner: USER.to_string(),
            amount: Uint128::from(300_000u128),
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::ScaledBalance {
            address: USER.to_string(),
        },
    )
    .unwrap();
    let res: BalanceResponse = from_json(bin).unwrap();
    assert_eq!(res.balance, Uint128::from(700_000u128));

    let info_bin = query(deps.as_ref(), mock_env(), QueryMsg::TokenInfo {}).unwrap();
    let info: TokenInfoResponse = from_json(info_bin).unwrap();
    assert_eq!(info.total_supply, Uint128::from(700_000u128));
}

#[test]
fn burn_from_rejected_when_not_minter() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1_000_000u128),
        },
    )
    .unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::BurnFrom {
            owner: USER.to_string(),
            amount: Uint128::from(100_000u128),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NotAuthorizedError { .. }));
}

// --- Storage: full decrease removes zero balance entries ---

#[test]
fn burn_removes_balance_entry_when_balance_reaches_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let minter = Addr::unchecked(MINTER);
    let amount = Uint128::from(42u128);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&minter, &[]),
        ExecuteMsg::Mint {
            recipient: MINTER.to_string(),
            amount,
        },
    )
    .unwrap();

    assert!(BALANCES
        .may_load(deps.as_ref().storage, minter.clone())
        .unwrap()
        .is_some());

    execute(
        deps.as_mut(),
        env,
        message_info(&minter, &[]),
        ExecuteMsg::Burn { amount },
    )
    .unwrap();

    assert!(
        BALANCES
            .may_load(deps.as_ref().storage, minter)
            .unwrap()
            .is_none(),
        "expected balance key removed when burn reaches zero"
    );
}

#[test]
fn transfer_removes_sender_balance_entry_when_balance_reaches_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = InstantiateMsg {
        pool_address: Some(POOL.to_string()),
        ..default_instantiate_msg()
    };
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    let env = mock_env();
    let pool = Addr::unchecked(POOL);
    let amount = Uint128::from(99u128);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: POOL.to_string(),
            amount,
        },
    )
    .unwrap();

    assert!(BALANCES
        .may_load(deps.as_ref().storage, pool.clone())
        .unwrap()
        .is_some());

    execute(
        deps.as_mut(),
        env,
        message_info(&pool, &[]),
        ExecuteMsg::Transfer {
            recipient: USER.to_string(),
            amount,
        },
    )
    .unwrap();

    assert!(
        BALANCES
            .may_load(deps.as_ref().storage, pool)
            .unwrap()
            .is_none(),
        "expected sender balance key removed when transfer reaches zero"
    );
}

#[test]
fn send_removes_sender_balance_entry_when_balance_reaches_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = InstantiateMsg {
        pool_address: Some(POOL.to_string()),
        ..default_instantiate_msg()
    };
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    let env = mock_env();
    let user = Addr::unchecked(USER);
    let amount = Uint128::from(77u128);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount,
        },
    )
    .unwrap();

    assert!(BALANCES
        .may_load(deps.as_ref().storage, user.clone())
        .unwrap()
        .is_some());

    execute(
        deps.as_mut(),
        env,
        message_info(&user, &[]),
        ExecuteMsg::Send {
            contract: POOL.to_string(),
            amount,
            msg: to_json_binary(&()).unwrap(),
        },
    )
    .unwrap();

    assert!(
        BALANCES
            .may_load(deps.as_ref().storage, user)
            .unwrap()
            .is_none(),
        "expected sender balance key removed when send reaches zero"
    );
}

// --- Auth: only owner can UpdateConfig; ownership via cw-ownable ---

#[test]
fn update_ownership_transfer_succeeds_after_accept() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_ADMIN.to_string(),
            expiry: None,
        }),
    )
    .expect("propose transfer");

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(NEW_ADMIN), &[]),
        ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
    )
    .expect("accept");

    let o = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(o.owner, Some(Addr::unchecked(NEW_ADMIN)));
}

#[test]
fn update_ownership_transfer_rejected_when_not_owner() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_ADMIN.to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::Ownership(cw_ownable::OwnershipError::NotOwner)
    ));
}

#[test]
fn update_ownership_rejected_with_funds() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(ADMIN), &[coin(1, "nhash")]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: NEW_ADMIN.to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidFundsError { .. }));
}

#[test]
fn update_ownership_renounce_rejected() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(ADMIN), &[]),
        ExecuteMsg::UpdateOwnership(Action::RenounceOwnership),
    )
    .unwrap_err();
    match err {
        ContractError::IllegalArgumentError { message } => {
            assert_eq!(message, "Renouncing contract ownership is not supported");
        }
        _ => panic!("expected IllegalArgument, got {:?}", err),
    }
}

#[test]
fn update_ownership_transfer_rejected_with_invalid_new_owner() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(ADMIN), &[]),
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: "pb1q3xhmqrjukjuhmccy4p6xza6q0uxwclled4wrf".to_string(),
            expiry: None,
        }),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::Ownership(cw_ownable::OwnershipError::Std(_))
    ));
}

#[test]
fn update_config_rejected_when_not_owner() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::UpdateConfig {
            minter: Some(POOL.to_string()),
            pool_address: Some(POOL.to_string()),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == UPDATE_CONFIG_ASSERT_OWNER_ERR
    ));
}

// --- Happy path: UpdateConfig then Balance returns underlying ---

#[test]
fn update_config_sets_pool_then_balance_returns_underlying() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1_000_000u128),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        ExecuteMsg::UpdateConfig {
            minter: Some(MINTER.to_string()),
            pool_address: Some(POOL.to_string()),
        },
    )
    .unwrap();
    mock_pool_reserve(&mut deps.querier, POOL, "1.02");

    let bin = query(
        deps.as_ref(),
        env,
        QueryMsg::Balance {
            address: USER.to_string(),
        },
    )
    .unwrap();
    let res: BalanceResponse = from_json(bin).unwrap();
    assert_eq!(res.balance, Uint128::from(1_020_000u128)); // 1_000_000 * 1.02
}

// --- Instantiate validation (cw20-base style, see CW20_AUDIT.md) ---

#[test]
fn instantiate_rejects_name_too_short() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.name = "ab".to_string();
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn instantiate_rejects_name_too_long() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.name = "a".repeat(51);
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn instantiate_rejects_symbol_too_short() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.symbol = "AB".to_string();
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn instantiate_rejects_symbol_too_long() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.symbol = "A".repeat(13);
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn instantiate_rejects_symbol_invalid_char() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.symbol = "AB@".to_string();
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn instantiate_rejects_decimals_over_18() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.decimals = 19;
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

// --- Query: AllAccounts (pagination) ---

#[test]
fn query_all_accounts_start_after_excludes_that_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    let a = deps.api.addr_make("all_a").to_string();
    let b = deps.api.addr_make("all_b").to_string();
    let c = deps.api.addr_make("all_c").to_string();

    for recipient in [&a, &b, &c] {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(MINTER), &[]),
            ExecuteMsg::Mint {
                recipient: recipient.clone(),
                amount: Uint128::one(),
            },
        )
        .unwrap();
    }

    let first_page = fetch_all_accounts(&deps, None, Some(1));
    assert_eq!(first_page.accounts.len(), 1);
    let first = first_page.accounts[0].clone();

    let after_first = fetch_all_accounts(&deps, Some(first.clone()), Some(10));
    assert!(
        !after_first.accounts.contains(&first),
        "exclusive start_after must omit the given address"
    );

    let full = fetch_all_accounts(&deps, None, Some(100));
    assert_eq!(full.accounts.len(), 3);
    assert_eq!(1 + after_first.accounts.len(), full.accounts.len());
}

#[test]
fn query_all_accounts_none_start_after_begins_at_smallest_key() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    let recipients: Vec<String> = ["begin_a", "begin_b"]
        .map(|label| deps.api.addr_make(label).to_string())
        .into();
    for recipient in recipients {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(MINTER), &[]),
            ExecuteMsg::Mint {
                recipient,
                amount: Uint128::one(),
            },
        )
        .unwrap();
    }

    let first_only = fetch_all_accounts(&deps, None, Some(1));
    let full = fetch_all_accounts(&deps, None, Some(100));
    assert_eq!(first_only.accounts[0], full.accounts[0]);
}

#[test]
fn query_all_accounts_limit_reduces_page_size() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    let recipients: Vec<String> = (0..5)
        .map(|i| deps.api.addr_make(&format!("lim{i}")).to_string())
        .collect();
    for recipient in recipients {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(MINTER), &[]),
            ExecuteMsg::Mint {
                recipient,
                amount: Uint128::one(),
            },
        )
        .unwrap();
    }

    let page = fetch_all_accounts(&deps, None, Some(2));
    assert_eq!(page.accounts.len(), 2);
}

#[test]
fn query_all_accounts_none_limit_defaults_to_default_page_size() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    let recipients: Vec<String> = (0..=DEFAULT_ALL_ACCOUNTS_PAGE_SIZE + 1)
        .map(|i| deps.api.addr_make(&format!("def{i}")).to_string())
        .collect();
    for recipient in recipients {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(MINTER), &[]),
            ExecuteMsg::Mint {
                recipient,
                amount: Uint128::one(),
            },
        )
        .unwrap();
    }

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::AllAccounts {
            start_after: None,
            limit: None,
        },
    )
    .unwrap();
    let res: AllAccountsResponse = from_json(bin).unwrap();
    assert_eq!(res.accounts.len(), DEFAULT_ALL_ACCOUNTS_PAGE_SIZE as usize);
}

#[test]
fn query_all_accounts_explicit_limit_capped_at_max() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    let count = MAX_ALL_ACCOUNTS_PAGE_SIZE + 5;
    let recipients: Vec<String> = (0..count)
        .map(|i| deps.api.addr_make(&format!("cap{i}")).to_string())
        .collect();
    for recipient in recipients {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(MINTER), &[]),
            ExecuteMsg::Mint {
                recipient,
                amount: Uint128::one(),
            },
        )
        .unwrap();
    }

    let page = fetch_all_accounts(&deps, None, Some(u32::MAX));
    assert_eq!(page.accounts.len(), MAX_ALL_ACCOUNTS_PAGE_SIZE as usize);
}

#[test]
fn query_all_accounts_omits_zero_balances() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    let env = mock_env();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(100u128),
        },
    )
    .unwrap();

    assert!(fetch_all_accounts(&deps, None, Some(100))
        .accounts
        .contains(&USER.to_string()));

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::BurnFrom {
            owner: USER.to_string(),
            amount: Uint128::from(100u128),
        },
    )
    .unwrap();

    assert!(!fetch_all_accounts(&deps, None, Some(100))
        .accounts
        .contains(&USER.to_string()));
}

// --- Send only to pool; Transfer only from pool (to non-pool) ---

#[test]
fn user_transfer_to_pool_fails() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.pool_address = Some(POOL.to_string());
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Transfer {
            recipient: POOL.to_string(),
            amount: Uint128::from(500u128),
        },
    );
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ContractError::IllegalArgumentError { .. }
    ));
}

#[test]
fn pool_transfer_to_user_succeeds_when_pool_configured() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.pool_address = Some(POOL.to_string());
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: POOL.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(POOL), &[]),
        ExecuteMsg::Transfer {
            recipient: USER.to_string(),
            amount: Uint128::from(500u128),
        },
    );
    assert!(result.is_ok());
}

#[test]
fn transfer_to_non_pool_fails() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.pool_address = Some(POOL.to_string());
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Transfer {
            recipient: ADMIN.to_string(),
            amount: Uint128::from(500u128),
        },
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}

#[test]
fn transfer_fails_when_pool_not_configured() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    instantiate_default(&mut deps);
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Transfer {
            recipient: POOL.to_string(),
            amount: Uint128::from(500u128),
        },
    );
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ContractError::PoolNotConfigured
    ));
}

#[test]
fn send_to_pool_succeeds_when_pool_configured() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.pool_address = Some(POOL.to_string());
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Send {
            contract: POOL.to_string(),
            amount: Uint128::from(500u128),
            msg: to_json_binary(&()).unwrap(),
        },
    );
    assert!(result.is_ok());
}

#[test]
fn send_to_non_pool_fails() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.pool_address = Some(POOL.to_string());
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(ADMIN), &[]),
        msg,
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(MINTER), &[]),
        ExecuteMsg::Mint {
            recipient: USER.to_string(),
            amount: Uint128::from(1000u128),
        },
    )
    .unwrap();

    let result = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(USER), &[]),
        ExecuteMsg::Send {
            contract: ADMIN.to_string(),
            amount: Uint128::from(500u128),
            msg: to_json_binary(&()).unwrap(),
        },
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, ContractError::IllegalArgumentError { .. }));
}
