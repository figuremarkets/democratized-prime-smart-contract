//! Read-only quote of the pre-write `Liquidate` checks.
//!
//! Indexes are accrued in memory. Nothing is stored. `LiquidationAccess` and the
//! permissionless owner gate are not applied.

use crate::execute::liquidate::{preview_liquidation, LiquidationPreview};
use crate::model::DirectedFillQuoteResponseV1;
use crate::model::QueryError;
use crate::storage::get_contract_state_v1;
use crate::utils::compute_effective_reserve;
use cosmwasm_std::{to_json_binary, Binary, Deps, Env, Uint128};
use std::collections::BTreeMap;

pub fn query_directed_fill_quote(
    deps: Deps,
    env: Env,
    borrower: String,
    collateral_to_seize: &BTreeMap<String, Uint128>,
    min_collateral_out: Option<&BTreeMap<String, Uint128>>,
    sent: Uint128,
) -> Result<Binary, QueryError> {
    let contract = get_contract_state_v1(deps.storage)?;
    let reserve = compute_effective_reserve(deps.storage, env.block.time, &contract.rate_params)?;
    let plan = preview_liquidation(&LiquidationPreview {
        storage: deps.storage,
        querier: &deps.querier,
        block_time: env.block.time,
        api: deps.api,
        contract: &contract,
        borrow_index: reserve.borrow_index,
        borrower: &borrower,
        signed_seize: collateral_to_seize,
        min_collateral_out,
        sent_u128: sent.u128(),
        owner_sender: None,
    })?;
    let response = DirectedFillQuoteResponseV1 {
        debt_payoff: Uint128::new(plan.debt_payoff),
        actual_repay_underlying: Uint128::new(plan.actual_repay_underlying),
        refund: Uint128::new(sent.u128().saturating_sub(plan.actual_repay_underlying)),
        collateral_to_seize: plan.collateral_to_seize,
        seized_value_usd: plan.seized_value_usd,
        min_collateral_value_required: plan.min_collateral_value_required,
        max_collateral_value_allowed: plan.max_collateral_value_allowed,
        directed_fill: plan.directed_fill,
    };
    Ok(to_json_binary(&response)?)
}
