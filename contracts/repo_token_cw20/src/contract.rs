#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response};
use democratized_prime_lib::common::migrate_contract;
use democratized_prime_lib::common::LegacyMigration;

use crate::constants::CONTRACT_NAME;
use crate::constants::CONTRACT_VERSION;
use crate::error::ContractError;
use crate::execute;
use crate::instantiate;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::query;
use crate::state::Config;
use crate::state::CONFIG;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    instantiate::instantiate(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    execute::execute(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    query::query(deps, env, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    migrate_contract::<Config>(
        deps.storage,
        CONTRACT_NAME,
        CONTRACT_VERSION,
        Some(LegacyMigration {
            item: &CONFIG,
            api: deps.api,
        }),
    )
}
