//! Borrower adjusts or cancels the directed-sale outstanding map on their own position.
//! Signed amounts add or subtract (clamp at zero). Fill is `Liquidate`, not this message.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_ADJUSTMENTS_JSON, ATTRIBUTE_APPLIED_JSON, ATTRIBUTE_BORROWER,
    ATTRIBUTE_COLLATERAL_JSON, ATTRIBUTE_EXPIRES_AT, ATTRIBUTE_VERSION,
};
use crate::model::directed_sale::DirectedSaleOfferV1;
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::storage::{
    clear_directed_sale_offer, get_borrower_collateral, get_contract_state_v1,
    get_directed_sale_offer, get_directed_sale_version, get_scaled_borrow, set_directed_sale_offer,
    set_directed_sale_version,
};
use cosmwasm_std::{ensure, DepsMut, Env, Int128, MessageInfo, Response, Timestamp, Uint128};
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "adjust_directed_collateral_sale";

/// Adjust the sender's outstanding offer, or cancel it when `adjustments` normalizes to empty.
/// Does not check borrower attributes, so a borrower can always cancel or decrease.
pub fn adjust_directed_collateral_sale(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    adjustments: &BTreeMap<String, Int128>,
    expires_at: Timestamp,
    version: u64,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let borrower = info.sender.as_str();
    let stored_version = get_directed_sale_version(deps.storage, borrower)?;
    let expected = stored_version
        .checked_add(1)
        .ok_or_else(|| illegal_state("directed sale version overflow"))?;
    ensure!(
        version == expected,
        illegal_argument(format!("version must be {}, got {}", expected, version))
    );

    let requested = normalize_adjustments(adjustments);
    if requested.is_empty() {
        clear_directed_sale_offer(deps.storage, borrower)?;
        set_directed_sale_version(deps.storage, borrower, version)?;
        return Ok(response(
            borrower,
            &requested,
            &BTreeMap::new(),
            &BTreeMap::new(),
            "0",
            version,
        ));
    }

    let any_increase = requested.values().any(|d| d.i128() > 0);
    if any_increase {
        ensure!(
            expires_at > env.block.time,
            illegal_argument("expires_at must be in the future")
        );
        let scaled = get_scaled_borrow(deps.storage, borrower)?;
        ensure!(
            scaled > 0,
            illegal_argument("Borrower has no debt (cannot list a sale with nothing to repay)")
        );
    }

    let current_offer = get_directed_sale_offer(deps.storage, borrower)?;
    let mut outstanding: BTreeMap<String, Uint128> = current_offer
        .as_ref()
        .map(|o| o.amounts.clone())
        .unwrap_or_default();
    let mut applied: BTreeMap<String, Int128> = BTreeMap::new();

    let supported_ids: HashSet<_> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| a.asset_id.as_str())
        .collect();
    let balances = get_borrower_collateral(deps.storage, borrower)?;

    for (asset_id, delta) in &requested {
        let signed = delta.i128();
        ensure!(
            signed != i128::MIN,
            illegal_argument(format!("Adjustment magnitude too large for {}", asset_id))
        );
        if signed > 0 {
            ensure!(
                supported_ids.contains(asset_id.as_str()),
                illegal_argument(format!("Unsupported collateral asset: {}", asset_id))
            );
            let add = signed as u128;
            let have = outstanding.get(asset_id).map(|a| a.u128()).unwrap_or(0);
            let new_amt = have.checked_add(add).ok_or_else(|| {
                illegal_argument(format!("Outstanding overflow for {}", asset_id))
            })?;
            let bal = *balances.amounts.get(asset_id.as_str()).unwrap_or(&0);
            ensure!(
                bal >= new_amt,
                illegal_argument(format!(
                    "Insufficient collateral for {}: have {}, requested {}",
                    asset_id, bal, new_amt
                ))
            );
            outstanding.insert(asset_id.clone(), Uint128::new(new_amt));
            applied.insert(asset_id.clone(), *delta);
        } else {
            let mag = (-signed) as u128;
            let have = outstanding.get(asset_id).map(|a| a.u128()).unwrap_or(0);
            let take = have.min(mag);
            if take == 0 {
                outstanding.remove(asset_id);
                continue;
            }
            let new_amt = have - take;
            if new_amt == 0 {
                outstanding.remove(asset_id);
            } else {
                outstanding.insert(asset_id.clone(), Uint128::new(new_amt));
            }
            applied.insert(asset_id.clone(), Int128::from(-(take as i128)));
        }
    }

    if outstanding.is_empty() {
        clear_directed_sale_offer(deps.storage, borrower)?;
        set_directed_sale_version(deps.storage, borrower, version)?;
        return Ok(response(
            borrower,
            &requested,
            &applied,
            &outstanding,
            "0",
            version,
        ));
    }

    let offer_expires_at = if any_increase {
        expires_at
    } else {
        current_offer
            .as_ref()
            .map(|o| o.expires_at)
            .ok_or_else(|| illegal_state("outstanding directed sale missing expiry"))?
    };

    let offer = DirectedSaleOfferV1 {
        amounts: outstanding.clone(),
        expires_at: offer_expires_at,
    };
    set_directed_sale_offer(deps.storage, borrower, &offer)?;
    set_directed_sale_version(deps.storage, borrower, version)?;
    Ok(response(
        borrower,
        &requested,
        &applied,
        &outstanding,
        &offer_expires_at.nanos().to_string(),
        version,
    ))
}

fn normalize_adjustments(map: &BTreeMap<String, Int128>) -> BTreeMap<String, Int128> {
    map.iter()
        .filter(|(_, amt)| !amt.is_zero())
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

fn response(
    borrower: &str,
    requested: &BTreeMap<String, Int128>,
    applied: &BTreeMap<String, Int128>,
    outstanding: &BTreeMap<String, Uint128>,
    expires_at: &str,
    version: u64,
) -> Response {
    let adjustments_json: BTreeMap<String, String> = requested
        .iter()
        .map(|(id, amt)| (id.clone(), amt.to_string()))
        .collect();
    let applied_json: BTreeMap<String, String> = applied
        .iter()
        .map(|(id, amt)| (id.clone(), amt.to_string()))
        .collect();
    let collateral_json: BTreeMap<String, String> = outstanding
        .iter()
        .map(|(id, amt)| (id.clone(), amt.to_string()))
        .collect();
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, borrower)
        .add_attribute(
            ATTRIBUTE_ADJUSTMENTS_JSON,
            serde_json::to_string(&adjustments_json).unwrap_or_default(),
        )
        .add_attribute(
            ATTRIBUTE_APPLIED_JSON,
            serde_json::to_string(&applied_json).unwrap_or_default(),
        )
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        )
        .add_attribute(ATTRIBUTE_EXPIRES_AT, expires_at)
        .add_attribute(ATTRIBUTE_VERSION, version.to_string())
}
