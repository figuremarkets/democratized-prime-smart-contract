use cosmwasm_std::{
    ensure, to_json_binary, Addr, Binary, DepsMut, Env, MessageInfo, Response, Uint128, WasmMsg,
};
use cw20::Cw20ReceiveMsg;
use democratized_prime_lib::common::{assert_owner, update_ownership};
use serde::Serialize;

use crate::error::{illegal_argument, ContractError};
use crate::msg::ExecuteMsg;
use crate::state::{Config, TokenInfo, BALANCES, CONFIG, TOKEN_INFO};

/// Deducts `amount` from `owner` balance and from total supply.
/// Caller must enforce minter and non-zero amount.
/// Removes the owner entry when the updated balance is zero (no zero balances expected in storage).
fn apply_burn(
    deps: &mut DepsMut,
    owner: &Addr,
    amount: Uint128,
    insufficient_balance_msg: &str,
) -> Result<(), ContractError> {
    let updated_balance = BALANCES.update(
        deps.storage,
        owner.clone(),
        |b| -> Result<Uint128, ContractError> {
            let current = b.unwrap_or(Uint128::zero());
            ensure!(
                current >= amount,
                illegal_argument(insufficient_balance_msg)
            );
            current.checked_sub(amount).map_err(ContractError::from)
        },
    )?;
    if updated_balance.is_zero() {
        BALANCES.remove(deps.storage, owner.clone());
    }
    TOKEN_INFO.update(deps.storage, |t| -> Result<TokenInfo, ContractError> {
        Ok(TokenInfo {
            total_supply: t
                .total_supply
                .checked_sub(amount)
                .map_err(ContractError::from)?,
            ..t
        })
    })?;
    Ok(())
}

/// Wrapper so the receiving contract (e.g. pool_v2) sees ExecuteMsg::Receive(...) when it deserializes.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ReceiveWrapper {
    receive: Cw20ReceiveMsg,
}

fn only_minter(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    ensure!(
        sender == config.minter,
        ContractError::NotAuthorizedError {
            message: "only minter may mint or burn".to_string(),
        }
    );
    Ok(())
}

/// CW20 `Send` must target the pool only. The pool's `Receive` runs withdraw/transfer logic.
/// Plain `Transfer` to the pool would credit the pool's CW20 balance without that hook.
fn ensure_send_allowed(config: &Config, recipient: &Addr) -> Result<(), ContractError> {
    let pool = config
        .pool_address
        .as_ref()
        .ok_or(ContractError::PoolNotConfigured)?;
    ensure!(
        recipient == pool,
        illegal_argument(
            "Send is only allowed to the pool address; use Send (not Transfer) so Receive runs"
        )
    );
    Ok(())
}

/// Only the pool may `Transfer` (refunds and forwards after `Receive`). Holders move position via `Send` to the pool.
/// `Transfer` to the pool is forbidden: it would strand tokens on the pool's CW20 balance without `Receive`.
fn ensure_transfer_allowed(
    config: &Config,
    sender: &Addr,
    recipient: &Addr,
) -> Result<(), ContractError> {
    let pool = config
        .pool_address
        .as_ref()
        .ok_or(ContractError::PoolNotConfigured)?;
    ensure!(
        sender == pool,
        illegal_argument("Only the pool may Transfer repo tokens")
    );
    ensure!(
        recipient != pool,
        illegal_argument("Transfer to the pool address is not allowed; use Send so the pool's Receive handler runs")
    );
    Ok(())
}

pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => update_ownership(deps, env, info, action),
        ExecuteMsg::Mint { recipient, amount } => execute_mint(deps, env, info, recipient, amount),
        ExecuteMsg::Burn { amount } => execute_burn(deps, env, info, amount),
        ExecuteMsg::BurnFrom { owner, amount } => execute_burn_from(deps, info, owner, amount),
        ExecuteMsg::Transfer { recipient, amount } => {
            execute_transfer(deps, env, info, recipient, amount)
        }
        ExecuteMsg::Send {
            contract,
            amount,
            msg: send_msg,
        } => execute_send(deps, env, info, contract, amount, send_msg),
        ExecuteMsg::UpdateConfig {
            minter,
            pool_address,
        } => execute_update_config(deps, info, minter, pool_address),
    }
}

fn execute_mint(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    only_minter(&config, &info.sender)?;
    let recipient = deps.api.addr_validate(&recipient)?;
    ensure!(!amount.is_zero(), illegal_argument("zero mint"));
    BALANCES.update(
        deps.storage,
        recipient.clone(),
        |b| -> Result<Uint128, ContractError> {
            b.unwrap_or(Uint128::zero())
                .checked_add(amount)
                .map_err(ContractError::from)
        },
    )?;
    TOKEN_INFO.update(deps.storage, |t| -> Result<TokenInfo, ContractError> {
        Ok(TokenInfo {
            total_supply: t
                .total_supply
                .checked_add(amount)
                .map_err(ContractError::from)?,
            ..t
        })
    })?;
    Ok(Response::new()
        .add_attribute("action", "mint")
        .add_attribute("recipient", recipient.as_str())
        .add_attribute("amount", amount))
}

fn execute_burn(
    mut deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    only_minter(&config, &info.sender)?;
    ensure!(!amount.is_zero(), illegal_argument("zero burn"));
    apply_burn(
        &mut deps,
        &info.sender,
        amount,
        "insufficient balance to burn",
    )?;
    Ok(Response::new()
        .add_attribute("action", "burn")
        .add_attribute("amount", amount))
}

fn execute_burn_from(
    mut deps: DepsMut,
    info: MessageInfo,
    owner: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    only_minter(&config, &info.sender)?;
    ensure!(!amount.is_zero(), illegal_argument("zero burn from"));
    let owner_addr = deps.api.addr_validate(&owner)?;
    apply_burn(
        &mut deps,
        &owner_addr,
        amount,
        "insufficient balance to burn from",
    )?;
    Ok(Response::new()
        .add_attribute("action", "burn_from")
        .add_attribute("by", info.sender)
        .add_attribute("owner", owner)
        .add_attribute("amount", amount))
}

fn execute_transfer(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    recipient: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    ensure!(!amount.is_zero(), illegal_argument("zero transfer"));
    let recipient = deps.api.addr_validate(&recipient)?;
    let config = CONFIG.load(deps.storage)?;
    ensure_transfer_allowed(&config, &info.sender, &recipient)?;
    let updated_sender_balance = BALANCES.update(
        deps.storage,
        info.sender.clone(),
        |b| -> Result<Uint128, ContractError> {
            let current = b.unwrap_or(Uint128::zero());
            ensure!(
                current >= amount,
                illegal_argument(format!(
                    "insufficient balance [{}] to transfer [{}]",
                    current, amount
                ))
            );
            current.checked_sub(amount).map_err(ContractError::from)
        },
    )?;
    if updated_sender_balance.is_zero() {
        BALANCES.remove(deps.storage, info.sender.clone());
    }
    BALANCES.update(
        deps.storage,
        recipient.clone(),
        |b| -> Result<Uint128, ContractError> {
            b.unwrap_or(Uint128::zero())
                .checked_add(amount)
                .map_err(ContractError::from)
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "transfer")
        .add_attribute("from", info.sender.as_str())
        .add_attribute("to", recipient.as_str())
        .add_attribute("amount", amount))
}

fn execute_send(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    contract: String,
    amount: Uint128,
    msg: Binary,
) -> Result<Response, ContractError> {
    ensure!(!amount.is_zero(), illegal_argument("zero send"));
    let contract_addr = deps.api.addr_validate(&contract)?;
    let config = CONFIG.load(deps.storage)?;
    ensure_send_allowed(&config, &contract_addr)?;
    let updated_sender_balance = BALANCES.update(
        deps.storage,
        info.sender.clone(),
        |b| -> Result<Uint128, ContractError> {
            let current = b.unwrap_or(Uint128::zero());
            ensure!(
                current >= amount,
                illegal_argument("insufficient balance to send")
            );
            current.checked_sub(amount).map_err(ContractError::from)
        },
    )?;
    if updated_sender_balance.is_zero() {
        BALANCES.remove(deps.storage, info.sender.clone());
    }
    BALANCES.update(
        deps.storage,
        contract_addr.clone(),
        |b| -> Result<Uint128, ContractError> {
            b.unwrap_or(Uint128::zero())
                .checked_add(amount)
                .map_err(ContractError::from)
        },
    )?;
    let cw20_receive = Cw20ReceiveMsg {
        sender: info.sender.to_string(),
        amount,
        msg,
    };
    let exec = WasmMsg::Execute {
        contract_addr: contract,
        msg: to_json_binary(&ReceiveWrapper {
            receive: cw20_receive,
        })?,
        funds: vec![],
    };
    Ok(Response::new()
        .add_message(exec)
        .add_attribute("action", "send")
        .add_attribute("from", info.sender.as_str())
        .add_attribute("to", contract_addr.as_str())
        .add_attribute("amount", amount))
}

pub const UPDATE_CONFIG_ASSERT_OWNER_ERR: &str =
    "Only the contract owner may update token configuration";

fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    minter: Option<String>,
    pool_address: Option<String>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_owner(deps.storage, &info.sender, UPDATE_CONFIG_ASSERT_OWNER_ERR)?;
    let new_minter: Addr = match minter {
        Some(ref minter) => deps.api.addr_validate(minter.as_str())?,
        None => config.minter,
    };
    let new_pool_address: Option<Addr> = match pool_address {
        Some(ref pool_address) => Some(deps.api.addr_validate(pool_address.as_str())?),
        None => config.pool_address,
    };
    CONFIG.save(
        deps.storage,
        &Config {
            minter: new_minter.clone(),
            pool_address: new_pool_address,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "update_config")
        .add_attribute("minter", new_minter.as_str()))
}
