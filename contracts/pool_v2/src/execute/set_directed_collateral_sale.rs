//! Borrower sets, replaces, or cancels the single directed-sale offer on their own position.
//! An empty collateral map cancels. Fill is `Liquidate`, not this message.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON, ATTRIBUTE_EXPIRES_AT,
};
use crate::model::directed_sale::{normalize_seize_map, DirectedSaleOfferV1};
use crate::model::error::{illegal_argument, invalid_funds, ContractError};
use crate::storage::{
    clear_directed_sale_offer, get_borrower_collateral, get_contract_state_v1, get_scaled_borrow,
    set_directed_sale_offer,
};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response, Timestamp, Uint128};
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "set_directed_collateral_sale";

/// Set or replace the sender's offer, or cancel it when `collateral` normalizes to empty.
/// Does not check borrower attributes, so a borrower can always cancel.
pub fn set_directed_collateral_sale(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    collateral: &BTreeMap<String, Uint128>,
    expires_at: Timestamp,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let borrower = info.sender.as_str();
    let amounts = normalize_seize_map(collateral);
    if amounts.is_empty() {
        clear_directed_sale_offer(deps.storage, borrower)?;
        return Ok(response(borrower, &amounts, "0"));
    }

    ensure!(
        expires_at > env.block.time,
        illegal_argument("expires_at must be in the future")
    );
    let scaled = get_scaled_borrow(deps.storage, borrower)?;
    ensure!(
        scaled > 0,
        illegal_argument("Borrower has no debt (cannot list a sale with nothing to repay)")
    );
    let current = get_borrower_collateral(deps.storage, borrower)?;
    ensure!(
        !current.amounts.is_empty(),
        illegal_argument("Borrower has no collateral")
    );

    let supported_ids: HashSet<_> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| a.asset_id.as_str())
        .collect();
    for (asset_id, amount) in &amounts {
        ensure!(
            supported_ids.contains(asset_id.as_str()),
            illegal_argument(format!("Unsupported collateral asset: {}", asset_id))
        );
        let have = *current.amounts.get(asset_id.as_str()).unwrap_or(&0);
        ensure!(
            have >= amount.u128(),
            illegal_argument(format!(
                "Insufficient collateral for {}: have {}, requested {}",
                asset_id, have, amount
            ))
        );
    }

    let offer = DirectedSaleOfferV1 {
        amounts: amounts.clone(),
        expires_at,
    };
    set_directed_sale_offer(deps.storage, borrower, &offer)?;
    Ok(response(
        borrower,
        &amounts,
        &expires_at.nanos().to_string(),
    ))
}

fn response(borrower: &str, amounts: &BTreeMap<String, Uint128>, expires_at: &str) -> Response {
    let collateral_json: BTreeMap<String, String> = amounts
        .iter()
        .map(|(id, amt)| (id.clone(), amt.to_string()))
        .collect();
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, borrower)
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        )
        .add_attribute(ATTRIBUTE_EXPIRES_AT, expires_at)
}
