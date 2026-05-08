use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_LENDER, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::model::ReserveStateV1;
use crate::msg::repo_token::{RepoTokenExecuteMsg, RepoTokenQueryMsg, RepoTokenScaledBalanceQuery};
use crate::storage::{
    get_contract_state_v1, get_lender_require_commit_on_exit, set_reserve_state_v1,
};
use crate::utils::{
    reserve_totals_and_cash_u128, scaled_to_underlying_liquidity, underlying_to_scaled_liquidity,
    update_reserve_indexes, WithRates,
};
use cosmwasm_std::{
    ensure, to_json_binary, Addr, BankMsg, Coin, CosmosMsg, DepsMut, Empty, Env, MessageInfo,
    Response, Storage, Uint128, WasmMsg,
};
use cw20::{BalanceResponse, Cw20ExecuteMsg};
use democratized_prime_lib::common::assert_owner;
use provwasm_std::types::cosmos::authz::v1beta1::MsgExec;
use provwasm_std::types::cosmos::base::v1beta1::Coin as ProvCoin;
use provwasm_std::types::provenance::exchange::v1::MsgCommitFundsRequest;
use serde::Serialize;

/// Deduct scaled liquidity from reserve and persist. Shared by both withdraw paths.
fn deduct_scaled_liquidity_and_save(
    storage: &mut dyn Storage,
    reserve: &mut ReserveStateV1,
    scaled_to_burn: u128,
) -> Result<(), ContractError> {
    reserve.total_scaled_liquidity = reserve
        .total_scaled_liquidity
        .checked_sub(scaled_to_burn)
        .ok_or_else(|| illegal_state("underflow: total_scaled_liquidity - scaled_to_burn"))?;
    set_reserve_state_v1(storage, reserve)?;
    Ok(())
}

/// Build BankMsg::Send for lending denom. Shared by both withdraw paths.
fn bank_send_underlying(denom: &str, to_address: &str, amount: u128) -> BankMsg {
    BankMsg::Send {
        to_address: to_address.to_string(),
        amount: vec![Coin {
            denom: denom.to_string(),
            amount: Uint128::from(amount),
        }],
    }
}

/// Build WasmMsg::Execute to repo token contract. Shared by both withdraw paths (Burn vs BurnFrom).
fn wasm_execute_repo_token<T: Serialize>(
    contract_addr: &str,
    msg: &T,
) -> Result<WasmMsg, ContractError> {
    Ok(WasmMsg::Execute {
        contract_addr: contract_addr.to_string(),
        msg: to_json_binary(msg).map_err(ContractError::Std)?,
        funds: vec![],
    })
}

pub const ACTION: &str = "withdraw";
pub const ACTION_EXACT: &str = "withdraw_exact";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may withdraw on behalf of a lender";

// --- Owner withdraw on behalf of lender (ExecuteMsg::Withdraw): burn lender's repo token, send underlying to lender ---

/// Contract owner only: withdraw a lender's supply on their behalf. Queries the repo CW20 for the lender's
/// scaled balance, burns that (or a portion) via BurnFrom, updates total_scaled_liquidity, and sends
/// underlying to the lender. Does not check require_commit_on_exit. Optionally pass commit_funds: true
/// to emit MsgCommitFundsRequest when commit_market_id is set. Use when closing a pool.
pub fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    lender: String,
    amount: Option<Uint128>,
    commit_funds: Option<bool>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let repo_token = contract.repo_token_addr()?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let lender_addr = deps.api.addr_validate(lender.trim())?;

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    let (_total_liquidity, _total_borrow, cash) = reserve_totals_and_cash_u128(&reserve)?;

    let query_msg = RepoTokenQueryMsg {
        scaled_balance: RepoTokenScaledBalanceQuery {
            address: lender_addr.to_string(),
        },
    };
    let balance: BalanceResponse = deps
        .querier
        .query_wasm_smart(repo_token.to_string(), &query_msg)?;
    let scaled_balance = balance.balance.u128();
    ensure!(
        scaled_balance > 0,
        illegal_argument("Lender has no repo token balance")
    );

    let underlying_max = scaled_to_underlying_liquidity(scaled_balance, reserve.liquidity_index)?;
    let (scaled_to_burn, underlying_to_send) = match amount {
        Some(amt) => {
            ensure!(
                !amt.is_zero(),
                illegal_argument("Amount must be greater than zero")
            );
            ensure!(
                amt.u128() <= underlying_max,
                illegal_argument("Amount exceeds lender's supply")
            );
            ensure!(
                amt.u128() <= cash,
                illegal_argument("Insufficient liquidity: amount would exceed available cash")
            );
            let scaled = underlying_to_scaled_liquidity(amt.u128(), reserve.liquidity_index)?;
            let actual_underlying =
                scaled_to_underlying_liquidity(scaled, reserve.liquidity_index)?;
            (scaled, actual_underlying)
        }
        None => {
            ensure!(
                underlying_max <= cash,
                illegal_argument(
                    "Insufficient liquidity: lender supply would exceed available cash"
                )
            );
            let actual_underlying =
                scaled_to_underlying_liquidity(scaled_balance, reserve.liquidity_index)?;
            (scaled_balance, actual_underlying)
        }
    };

    deduct_scaled_liquidity_and_save(deps.storage, &mut reserve, scaled_to_burn)?;

    let burn_from_msg = RepoTokenExecuteMsg::BurnFrom {
        owner: lender_addr.to_string(),
        amount: Uint128::from(scaled_to_burn),
    };
    let burn_msg = wasm_execute_repo_token(repo_token.as_str(), &burn_from_msg)?;
    let send_msg = bank_send_underlying(
        &contract.lending_denom.name,
        lender_addr.as_str(),
        underlying_to_send,
    );

    let mut res = Response::new()
        .add_message(burn_msg)
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_LENDER, lender_addr.as_str())
        .add_attribute(ATTRIBUTE_AMOUNT, underlying_to_send.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_to_burn.to_string());

    if commit_funds == Some(true) {
        let market_id = contract.commit_market_id.ok_or_else(|| {
            illegal_argument(
                "commit_funds is true but commit_market_id is not configured on the contract",
            )
        })?;
        let funds: Vec<ProvCoin> = vec![contract.lending_denom.to_prov_coin(underlying_to_send)];
        let commit_msg: CosmosMsg<Empty> = MsgExec {
            grantee: env.contract.address.to_string(),
            msgs: vec![MsgCommitFundsRequest {
                account: lender_addr.to_string(),
                market_id,
                amount: funds,
                creation_fee: None,
                event_tag: format!("demo-prime:withdraw:{}:{}", lender_addr.as_str(), ACTION),
            }
            .to_any()],
        }
        .into();
        res = res.add_message(commit_msg);
    }

    res.attach_rates(&reserve, &contract.rate_params)
}

// --- User withdraw (Receive): burn sent repo token, send underlying to sender ---

/// Withdraw/withdraw_exact when repo token is sent via CW20 Receive. Burns received CW20, updates reserve, sends underlying; optionally refunds excess CW20.
/// When this lender has per-address "require commit on exit" set, commit_funds must be Some(true).
pub fn execute_withdraw_cw20(
    deps: DepsMut,
    env: Env,
    sender: Addr,
    received_scaled: u128,
    amount_underlying: Option<Uint128>,
    exact: bool,
    commit_funds: Option<bool>,
) -> Result<Response, ContractError> {
    ensure!(
        received_scaled > 0,
        illegal_argument("Withdraw amount must be greater than zero")
    );
    let contract = get_contract_state_v1(deps.storage)?;
    let repo_token = contract.repo_token_addr()?;

    let effective_require = get_lender_require_commit_on_exit(deps.storage, &sender)?;
    ensure!(
        !effective_require || commit_funds == Some(true),
        illegal_argument(
            "Withdrawal requires commit_funds when commitment-on-exit is set; pass commit_funds: true to return funds to the committed market",
        )
    );
    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;

    let (_total_liquidity_u128, _total_borrow_u128, cash) = reserve_totals_and_cash_u128(&reserve)?;

    let (scaled_to_burn, underlying_to_send, action_name) = if exact {
        let underlying = scaled_to_underlying_liquidity(received_scaled, reserve.liquidity_index)?;
        ensure!(
            underlying <= cash,
            illegal_argument("Insufficient liquidity: scaled amount would exceed available cash")
        );
        (received_scaled, underlying, ACTION_EXACT)
    } else {
        let amount =
            amount_underlying.ok_or_else(|| illegal_argument("Withdraw amount required"))?;
        ensure!(
            !amount.is_zero(),
            illegal_argument("Withdraw amount must be greater than zero")
        );
        ensure!(
            amount.u128() <= cash,
            illegal_argument("Insufficient liquidity: amount exceeds available cash")
        );
        let scaled = underlying_to_scaled_liquidity(amount.u128(), reserve.liquidity_index)?;
        ensure!(
            received_scaled >= scaled,
            illegal_argument(format!(
                "Insufficient repo token sent: need {} (scaled) for {} underlying, got {}",
                scaled, amount, received_scaled
            ))
        );
        // Send the underlying value of the scaled amount we burn, not the requested amount.
        // floor(amount/index)*index can be < amount; sending amount would over-credit the user
        // and leak from the pool. Sending scaled_to_underlying(scaled) keeps accounting correct.
        let underlying_to_send = scaled_to_underlying_liquidity(scaled, reserve.liquidity_index)?;
        (scaled, underlying_to_send, ACTION)
    };

    deduct_scaled_liquidity_and_save(deps.storage, &mut reserve, scaled_to_burn)?;

    let burn_msg = wasm_execute_repo_token(
        repo_token.as_str(),
        &Cw20ExecuteMsg::Burn {
            amount: Uint128::from(scaled_to_burn),
        },
    )?;
    let send_msg = bank_send_underlying(
        &contract.lending_denom.name,
        sender.as_str(),
        underlying_to_send,
    );

    let mut res = Response::new()
        .add_message(burn_msg)
        .add_message(send_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, action_name)
        .add_attribute(ATTRIBUTE_LENDER, sender.as_str())
        .add_attribute(ATTRIBUTE_AMOUNT, underlying_to_send.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_to_burn.to_string());

    // When commit_funds is true, require commit_market_id and emit MsgCommitFundsRequest.
    if commit_funds == Some(true) {
        let market_id = contract.commit_market_id.ok_or_else(|| {
            illegal_argument(
                "commit_funds is true but commit_market_id is not configured on the contract",
            )
        })?;
        let funds: Vec<ProvCoin> = vec![contract.lending_denom.to_prov_coin(underlying_to_send)];
        let commit_msg: CosmosMsg<Empty> = MsgExec {
            grantee: env.contract.address.to_string(),
            msgs: vec![MsgCommitFundsRequest {
                account: sender.to_string(),
                market_id,
                amount: funds,
                creation_fee: None,
                event_tag: format!("demo-prime:withdraw:{}:{}", sender.as_str(), action_name),
            }
            .to_any()],
        }
        .into();
        res = res.add_message(commit_msg);
    }

    if !exact && received_scaled > scaled_to_burn {
        let refund = Uint128::new(received_scaled)
            .checked_sub(Uint128::new(scaled_to_burn))?
            .u128();
        let transfer_msg = wasm_execute_repo_token(
            repo_token.as_str(),
            &Cw20ExecuteMsg::Transfer {
                recipient: sender.to_string(),
                amount: Uint128::from(refund),
            },
        )?;
        res = res.add_message(transfer_msg);
    }

    res.attach_rates(&reserve, &contract.rate_params)
}
