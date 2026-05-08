//! Contract owner only: pro-rata supplier haircut via `liquidity_index` and reduce `deficit_underlying`.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_AMOUNT, ATTRIBUTE_DEFICIT_UNDERLYING};
use crate::model::error::{illegal_argument, illegal_state, invalid_funds, ContractError};
use crate::storage::{get_contract_state_v1, set_reserve_state_v1};
use crate::utils::{apply_pro_rata_liquidity_index_haircut, update_reserve_indexes, WithRates};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response, Uint128};
use democratized_prime_lib::common::assert_owner;

pub const ACTION: &str = "socialize_deficit";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may socialize deficit";

/// Applies `min(max_amount, deficit_underlying)` via [`apply_pro_rata_liquidity_index_haircut`].
pub fn socialize_deficit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    max_amount: Uint128,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let mut reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;
    let requested = max_amount.u128();
    ensure!(
        requested > 0,
        illegal_argument("max_amount must be positive")
    );
    let amt = requested.min(reserve.deficit_underlying);
    ensure!(
        amt > 0,
        illegal_state("Reserve has no deficit to socialize")
    );

    apply_pro_rata_liquidity_index_haircut(&mut reserve, amt)?;
    // Haircut does not change `deficit_underlying`; subtract the applied slice (≤ remaining deficit).
    reserve.deficit_underlying = reserve
        .deficit_underlying
        .checked_sub(amt)
        .ok_or_else(|| illegal_state("deficit_underlying underflow"))?;

    set_reserve_state_v1(deps.storage, &reserve)?;

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(ATTRIBUTE_AMOUNT, amt.to_string())
        .add_attribute(
            ATTRIBUTE_DEFICIT_UNDERLYING,
            reserve.deficit_underlying.to_string(),
        )
        .attach_rates(&reserve, &contract.rate_params)
}
