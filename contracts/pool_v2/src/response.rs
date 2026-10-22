use cosmwasm_std::{Addr, CosmosMsg, Empty};

use crate::model::{
    error::{illegal_argument, ContractError},
    ContractStateV1,
};
use crate::utils::new_event_tag;
use provwasm_std::types::cosmos::authz::v1beta1::MsgExec;
use provwasm_std::types::cosmos::base::v1beta1::Coin as ProvCoin;
use provwasm_std::types::provenance::exchange::v1::MsgCommitFundsRequest;

/// Generate a commitment message.
pub fn new_commit_message(
    contract_state: &ContractStateV1,
    contract_address: &Addr,
    action: &str,
    sub_action: &str,
    account: &Addr,
    funds: Vec<ProvCoin>,
) -> Result<CosmosMsg<Empty>, ContractError> {
    let market_id = contract_state
        .commit_market_id
        .ok_or_else(|| illegal_argument("commit_market_id is not configured on the contract"))?;

    Ok(MsgExec {
        grantee: contract_address.to_string(),
        msgs: vec![MsgCommitFundsRequest {
            account: account.to_string(),
            market_id,
            amount: funds,
            creation_fee: None,
            event_tag: new_event_tag(action, account, sub_action),
        }
        .to_any()],
    }
    .into())
}
