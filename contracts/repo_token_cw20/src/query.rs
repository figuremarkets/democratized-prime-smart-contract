use crate::constants::{DEFAULT_ALL_ACCOUNTS_PAGE_SIZE, MAX_ALL_ACCOUNTS_PAGE_SIZE};
use crate::error::ContractError;
use crate::msg::QueryMsg;
use crate::pool_query::query_liquidity_index;
use crate::state::{BALANCES, CONFIG, TOKEN_INFO};
use crate::utils::scaled_to_underlying_floor;
use cosmwasm_std::{to_json_binary, Addr, Binary, Deps, Env, Order, Uint128};
use cw20::{AllAccountsResponse, BalanceResponse, MinterResponse, TokenInfoResponse};
use cw_ownable::get_ownership;
use cw_storage_plus::Bound;

pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    match msg {
        QueryMsg::Ownership {} => Ok(to_json_binary(&get_ownership(deps.storage)?)?),
        QueryMsg::Balance { address } => {
            let balance = query_balance(deps, env, &address)?;
            to_json_binary(&balance).map_err(Into::into)
        }
        QueryMsg::ScaledBalance { address } => {
            let balance = query_scaled_balance(deps, &address)?;
            to_json_binary(&balance).map_err(Into::into)
        }
        QueryMsg::TokenInfo {} => {
            let info = query_token_info(deps, env)?;
            to_json_binary(&info).map_err(Into::into)
        }
        QueryMsg::Minter {} => {
            let minter = query_minter(deps)?;
            to_json_binary(&minter).map_err(Into::into)
        }
        QueryMsg::AllAccounts { start_after, limit } => {
            let accounts = query_all_accounts(deps, start_after, limit)?;
            to_json_binary(&accounts).map_err(Into::into)
        }
    }
}

/// Returns balance in underlying units when pool_address is set, else scaled.
fn query_balance(deps: Deps, _env: Env, address: &str) -> Result<BalanceResponse, ContractError> {
    let addr = deps.api.addr_validate(address)?;
    let scaled = BALANCES
        .may_load(deps.storage, addr)?
        .unwrap_or(Uint128::zero());
    let config = CONFIG.load(deps.storage)?;
    let amount = if let Some(ref pool_address) = config.pool_address {
        let index = query_liquidity_index(&deps.querier, pool_address)?;
        let underlying = scaled_to_underlying_floor(scaled.u128(), index)?;
        Uint128::from(underlying)
    } else {
        scaled
    };
    Ok(BalanceResponse { balance: amount })
}

/// Returns raw scaled balance (for pool admin Withdraw on behalf of lender).
/// Missing entries are treated as zero; no zero balances are expected to be stored.
pub fn query_scaled_balance(deps: Deps, address: &str) -> Result<BalanceResponse, ContractError> {
    let addr = deps.api.addr_validate(address)?;
    let scaled = BALANCES
        .may_load(deps.storage, addr)?
        .unwrap_or(Uint128::zero());
    Ok(BalanceResponse { balance: scaled })
}

/// Returns TokenInfo; total_supply in underlying when pool_address is set.
fn query_token_info(deps: Deps, _env: Env) -> Result<TokenInfoResponse, ContractError> {
    let info = TOKEN_INFO.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let total_supply = if let Some(ref pool) = config.pool_address {
        let pool_addr = deps.api.addr_validate(pool.as_str())?;
        let index = query_liquidity_index(&deps.querier, &pool_addr)?;
        let underlying = scaled_to_underlying_floor(info.total_supply.u128(), index)?;
        Uint128::from(underlying)
    } else {
        info.total_supply
    };
    Ok(TokenInfoResponse {
        name: info.name,
        symbol: info.symbol,
        decimals: info.decimals,
        total_supply,
    })
}

fn query_minter(deps: Deps) -> Result<MinterResponse, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    Ok(MinterResponse {
        minter: config.minter.to_string(),
        cap: None,
    })
}

fn query_all_accounts(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> Result<AllAccountsResponse, ContractError> {
    let start: Option<Bound<Addr>> = match start_after {
        Some(s) => Some(Bound::exclusive(deps.api.addr_validate(&s)?)),
        None => None,
    };
    let take = limit
        .unwrap_or(DEFAULT_ALL_ACCOUNTS_PAGE_SIZE)
        .min(MAX_ALL_ACCOUNTS_PAGE_SIZE) as usize;
    let accounts = BALANCES
        .range(deps.storage, start, None, Order::Ascending)
        .take(take)
        .map(|item| item.map(|(addr, _balance)| addr.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AllAccountsResponse { accounts })
}
