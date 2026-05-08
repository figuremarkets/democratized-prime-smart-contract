use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_REPO_TOKEN_ADDRESS, REPO_TOKEN_INSTANTIATE_REPLY_ID,
};
use crate::model::error::{illegal_state, ContractError};
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use cosmwasm_std::{DepsMut, Env, Reply, Response, SubMsgResponse, SubMsgResult};
use cw_utils::parse_instantiate_response_data;
use result_extensions::ResultExtensions;

const ACTION: &str = "repo_token_instantiated";

fn contract_addr_from_instantiate_reply(reply: &Reply) -> Result<String, ContractError> {
    let bytes: &[u8] = match &reply.result {
        SubMsgResult::Ok(SubMsgResponse { msg_responses, .. }) => {
            let response = msg_responses.first().ok_or_else(|| {
                illegal_state(
                    "repo token instantiate reply missing msg_responses (expected \
                     MsgInstantiateContractResponse)",
                )
            })?;
            response.value.as_slice()
        }
        SubMsgResult::Err(err) => {
            return Err(illegal_state(format!(
                "repo token instantiate submessage failed: {err}"
            )));
        }
    };
    let parsed = parse_instantiate_response_data(bytes)
        .map_err(|e| illegal_state(format!("repo token instantiate reply parse: {e}")))?;
    Ok(parsed.contract_address)
}

/// Binds `repo_token_cw20_address` after the repo token contract is instantiated via `SubMsg` (**`RepoTokenConfig::New`** only; existing-token path sets the address in the same `instantiate` entrypoint). The instantiate path uses `SubMsg::reply_always` so a failed Wasm instantiate is handled here and returns `Err`, reverting the whole transaction.
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    ensure_reply_id(&msg)?;
    let addr_str = contract_addr_from_instantiate_reply(&msg)?;
    let addr = deps.api.addr_validate(addr_str.trim())?;

    let mut state = get_contract_state_v1(deps.storage)?;
    if state.repo_token_cw20_address.is_some() {
        return Err(illegal_state(
            "repo token already bound; unexpected instantiate reply",
        ));
    }
    state.repo_token_cw20_address = Some(addr.clone());
    set_contract_state_v1(deps.storage, &state)?;

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_REPO_TOKEN_ADDRESS, addr.as_str())
        .to_ok()
}

fn ensure_reply_id(msg: &Reply) -> Result<(), ContractError> {
    if msg.id != REPO_TOKEN_INSTANTIATE_REPLY_ID {
        return Err(illegal_state(format!(
            "unexpected reply id {} (expected {})",
            msg.id, REPO_TOKEN_INSTANTIATE_REPLY_ID
        )));
    }
    Ok(())
}
