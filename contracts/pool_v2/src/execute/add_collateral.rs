use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON};
use crate::model::error::{illegal_argument, illegal_state, ContractError};
use crate::storage::{
    add_total_collateral, get_borrower_collateral, get_contract_state_v1, set_borrower_collateral,
};
use crate::utils::{validate_borrower_attrs, validate_borrower_collateral_type_limit};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response, Uint128};
use result_extensions::ResultExtensions;
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "add_collateral";

/// Add collateral to the sender's borrower position. Funds are taken from `info.funds`.
/// All denoms must be in the pool's supported collateral assets; total distinct
/// collateral types (existing + new) cannot exceed `max_borrower_collateral_types`.
///
/// No minimum add amount is enforced here. Collateral dust does not
/// affect pool-wide liquidity or indexes (unlike lend); oracle + haircut value
/// tiny amounts as negligible for borrow capacity; positions are bounded by
/// max_borrower_collateral_types. We reject zero-amount coins to avoid no-op state.
pub fn add_collateral(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    validate_borrower_attrs(
        &deps.querier,
        info.sender.as_str(),
        &contract.borrower_required_attrs,
    )?;

    let borrower = info.sender.to_string();
    let new_collateral = info.funds;

    ensure!(
        !new_collateral.is_empty(),
        illegal_argument("At least one collateral coin must be sent")
    );

    let supported_ids: HashSet<_> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| a.asset_id.as_str())
        .collect();

    for coin in &new_collateral {
        ensure!(
            !coin.amount.is_zero(),
            illegal_argument("Collateral amount must be positive")
        );
        ensure!(
            supported_ids.contains(coin.denom.as_str()),
            illegal_argument(format!("Unsupported collateral asset: {}", coin.denom))
        );
    }

    let mut current = get_borrower_collateral(deps.storage, &borrower)?;
    validate_borrower_collateral_type_limit(
        &new_collateral,
        &current,
        contract.max_borrower_collateral_types,
    )?;

    for coin in &new_collateral {
        let cur = current.amounts.get(&coin.denom).copied().unwrap_or(0);
        let new_amount = Uint128::from(cur)
            .checked_add(coin.amount)
            .map_err(|_| illegal_state("collateral overflow"))?;
        current
            .amounts
            .insert(coin.denom.clone(), new_amount.u128());
    }

    set_borrower_collateral(deps.storage, &borrower, &current)?;
    for coin in &new_collateral {
        add_total_collateral(deps.storage, &coin.denom, coin.amount.u128())?;
    }

    let collateral_json: BTreeMap<String, String> = new_collateral
        .iter()
        .map(|c| (c.denom.clone(), c.amount.to_string()))
        .collect();
    let res = Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, borrower)
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        );
    res.to_ok()
}
