//! Handles CW20 Receive: when users Send repo token to this contract, dispatch Withdraw or Transfer from the payload.

use super::transfer;
use super::withdraw;
use crate::model::error::{illegal_argument, ContractError};
use crate::msg::execute::Cw20ReceivePayload;
use crate::storage::get_contract_state_v1;
use cosmwasm_std::{ensure, from_json, DepsMut, Env, MessageInfo, Response};
use cw20::Cw20ReceiveMsg;

/// Entry point when users Send repo token to this contract via the CW20. Parses the payload and
/// dispatches to Withdraw, WithdrawExact, Transfer, or TransferExact. Only the repo token CW20
/// contract may call; sender in the message is the user who sent the tokens.
pub fn receive(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let repo_token = contract.repo_token_addr()?;
    ensure!(
        info.sender == repo_token,
        illegal_argument(format!(
            "Only the repo token CW20 contract {} can send to this pool",
            repo_token
        ))
    );
    let payload: Cw20ReceivePayload = from_json(cw20_msg.msg.as_slice())
        .map_err(|e| illegal_argument(format!("Invalid receive payload: {}", e)))?;
    let sender = deps.api.addr_validate(&cw20_msg.sender)?;
    let received = cw20_msg.amount.u128();

    match payload {
        Cw20ReceivePayload::Withdraw {
            amount,
            commit_funds,
        } => withdraw::execute_withdraw_cw20(
            deps,
            env,
            sender,
            received,
            Some(amount),
            false,
            commit_funds,
        ),
        Cw20ReceivePayload::WithdrawExact { commit_funds } => {
            withdraw::execute_withdraw_cw20(deps, env, sender, received, None, true, commit_funds)
        }
        Cw20ReceivePayload::Transfer { recipient, amount } => transfer::execute_transfer_cw20(
            deps,
            env,
            sender,
            received,
            recipient,
            Some(amount),
            false,
        ),
        Cw20ReceivePayload::TransferExact { recipient } => {
            transfer::execute_transfer_cw20(deps, env, sender, received, recipient, None, true)
        }
    }
}
