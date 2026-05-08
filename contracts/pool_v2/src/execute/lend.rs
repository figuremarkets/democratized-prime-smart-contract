use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_LENDER, ATTRIBUTE_SCALED_AMOUNT,
};
use crate::model::error::{illegal_argument, illegal_state, ContractError};
use crate::storage::{get_contract_state_v1, set_reserve_state_v1};
use crate::utils::{
    underlying_to_scaled_liquidity, update_reserve_indexes, validate_lender_attrs,
    validate_single_coin_denom, WithRates,
};
use cosmwasm_std::{ensure, to_json_binary, DepsMut, Env, MessageInfo, Response, Uint128, WasmMsg};
use cw20::Cw20ExecuteMsg;

/// Action name for attributes. "lend" matches pool terminology: user supplies
/// liquidity to the pool and receives repo tokens; the pool then allocates that liquidity to
/// borrowers.
pub const ACTION: &str = "lend";

/// Lend: user sends lending-denom coins; we mint repo token (scaled amount) to the user.
///
/// **Scaling:** The amount sent is in lending denom **base units** (e.g. 1 display unit = 10^6 or
/// 10^9 depending on the "u" / "n" prefix). We do not apply a separate decimals conversion for
/// the mint: we treat that amount as "underlying", convert to scaled via the liquidity index
/// (floor), and mint that many repo tokens. The book never credits more underlying than was
/// received; any sub-unit remainder stays in the pool.
/// Repo token balances are always in **scaled** units, not in lending-denom display units.
pub fn lend(deps: DepsMut, env: Env, info: MessageInfo) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let repo_token = contract.repo_token_addr()?;
    validate_lender_attrs(
        &deps.querier,
        info.sender.as_str(),
        &contract.lender_required_attrs,
    )?;
    let amount = validate_single_coin_denom(&info, &contract.lending_denom, contract.min_lend)?;
    let amount_u128 = amount.u128();

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    let scaled_delta = underlying_to_scaled_liquidity(amount_u128, reserve.liquidity_index)?;
    ensure!(
        scaled_delta > 0,
        illegal_argument(
            "Lend amount rounds to zero scaled liquidity at the current index; use a larger amount",
        )
    );

    // Mint repo token (scaled amount) via CW20.
    let mint_msg = WasmMsg::Execute {
        contract_addr: repo_token.to_string(),
        msg: to_json_binary(&Cw20ExecuteMsg::Mint {
            recipient: info.sender.to_string(),
            amount: Uint128::from(scaled_delta),
        })
        .map_err(ContractError::Std)?,
        funds: vec![],
    };

    reserve.total_scaled_liquidity = reserve
        .total_scaled_liquidity
        .checked_add(scaled_delta)
        .ok_or_else(|| illegal_state("overflow: total_scaled_liquidity + scaled_delta"))?;
    // Ensure the updated scaled total can still be materialized as underlying
    // (scaled * liquidity_index) before persisting state.
    reserve.total_liquidity()?;
    set_reserve_state_v1(deps.storage, &reserve)?;

    Response::new()
        .add_message(mint_msg)
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_LENDER, info.sender.as_str())
        .add_attribute(ATTRIBUTE_AMOUNT, amount.to_string())
        .add_attribute(ATTRIBUTE_SCALED_AMOUNT, scaled_delta.to_string())
        .attach_rates(&reserve, &contract.rate_params)
}
