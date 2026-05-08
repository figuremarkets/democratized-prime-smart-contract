use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_BORROWER, ATTRIBUTE_COLLATERAL_JSON};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::model::BorrowerCollateralV1;
use crate::storage::{
    get_borrower_collateral, get_contract_state_v1, get_scaled_borrow, set_borrower_collateral,
    subtract_total_collateral,
};
use crate::utils::{
    get_asset_prices_for_borrower, get_borrower_health, scaled_to_underlying_borrow,
    update_reserve_indexes, validate_borrower_attrs, validate_borrower_is_healthy, WithRates,
};
use cosmwasm_std::{ensure, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Uint128};
use std::collections::{BTreeMap, HashSet};

pub const ACTION: &str = "remove_collateral";

/// Remove collateral from the sender's borrower position. Amounts are specified in `to_remove`
/// (denom -> amount). Coins are sent back to the sender. Removal is only allowed if the resulting
/// position still passes health checks (LTV remains below margin rate).
pub fn remove_collateral(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    to_remove: &BTreeMap<String, Uint128>,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));
    validate_borrower_attrs(
        &deps.querier,
        info.sender.as_str(),
        &contract.borrower_required_attrs,
    )?;

    ensure!(
        !to_remove.is_empty(),
        illegal_argument("At least one collateral amount to remove must be specified")
    );

    let supported_ids: HashSet<_> = contract
        .supported_collateral_assets
        .iter()
        .map(|a| a.asset_id.as_str())
        .collect();

    let borrower = info.sender.to_string();
    let current = get_borrower_collateral(deps.storage, &borrower)?;

    let mut new_amounts = current.amounts.clone();
    let mut send_coins: Vec<Coin> = Vec::with_capacity(to_remove.len());

    for (denom, amount) in to_remove {
        ensure!(
            !amount.is_zero(),
            illegal_argument(format!("Remove amount for {} must be positive", denom))
        );
        ensure!(
            supported_ids.contains(denom.as_str()),
            illegal_argument(format!("Unsupported collateral asset: {}", denom))
        );
        let cur = *new_amounts.get(denom.as_str()).unwrap_or(&0);
        let remove_u128 = amount.u128();
        ensure!(
            cur >= remove_u128,
            illegal_argument(format!(
                "Insufficient collateral for {}: have {}, requested {}",
                denom, cur, remove_u128
            ))
        );
        let remaining = cur.checked_sub(remove_u128).ok_or_else(|| {
            illegal_state("underflow: collateral remaining (cur - remove) for asset")
        })?;
        if remaining > 0 {
            new_amounts.insert(denom.clone(), remaining);
        } else {
            new_amounts.remove(denom.as_str());
        }
        send_coins.push(Coin {
            denom: denom.clone(),
            amount: *amount,
        });
    }

    let (debt_underlying, reserve) = {
        let reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
        let scaled = get_scaled_borrow(deps.storage, info.sender.as_str())?;
        let debt = scaled_to_underlying_borrow(scaled, reserve.borrow_index)?;
        (debt, reserve)
    };

    let new_collateral = BorrowerCollateralV1 {
        amounts: new_amounts,
    };

    if debt_underlying > 0 {
        let asset_prices = get_asset_prices_for_borrower(
            &deps.querier,
            &env.block.time,
            &contract,
            &new_collateral,
        )?;
        let (health, loan_to_value) = get_borrower_health(
            &contract,
            &contract.supported_collateral_assets,
            &asset_prices,
            &new_collateral,
            Uint128::from(debt_underlying),
        )?;
        validate_borrower_is_healthy(health, loan_to_value, &contract)?;
    }

    set_borrower_collateral(deps.storage, &borrower, &new_collateral)?;
    for (denom, amount) in to_remove {
        subtract_total_collateral(deps.storage, denom, amount.u128())?;
    }

    let collateral_json: std::collections::BTreeMap<String, String> = to_remove
        .iter()
        .map(|(d, a)| (d.clone(), a.to_string()))
        .collect();
    let res = Response::new()
        .add_message(BankMsg::Send {
            to_address: info.sender.to_string(),
            amount: send_coins,
        })
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_BORROWER, borrower.as_str())
        .add_attribute(
            ATTRIBUTE_COLLATERAL_JSON,
            serde_json::to_string(&collateral_json).unwrap_or_default(),
        );
    res.attach_rates(&reserve, &contract.rate_params)
}
