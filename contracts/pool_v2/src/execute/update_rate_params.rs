//! Contract owner only: update interest rate params (kink model). Full replacement; validated same as at instantiate.
//! Accrues reserve indexes to current block time with the *old* params before applying the new ones,
//! so the new curve applies only from this block onward.

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::model::error::{invalid_funds, ContractError};
use crate::model::RateParamsV1;
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
use crate::utils::{update_reserve_indexes, WithRates};
use cosmwasm_std::{ensure, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::assert_owner;

pub const ACTION: &str = "update_rate_params";
pub const ASSERT_OWNER_ERR: &str = "Only the contract owner may update rate parameters";

/// Update rate params. Contract owner only; no funds. Accrues reserve to current block with current (old)
/// params, then replaces rate_params so the new curve applies from this block onward.
pub fn update_rate_params(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    rate_params: RateParamsV1,
) -> Result<Response, ContractError> {
    let mut contract = get_contract_state_v1(deps.storage)?;
    assert_owner(deps.storage, &info.sender, ASSERT_OWNER_ERR)?;
    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    rate_params.validate()?;

    // Accrue indexes to now with old params so the new params apply only from this block onward.
    let reserve = update_reserve_indexes(deps.storage, &env, &contract.rate_params)?;

    contract.rate_params = rate_params;
    set_contract_state_v1(deps.storage, &contract)?;

    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .attach_rates(&reserve, &contract.rate_params)
}
