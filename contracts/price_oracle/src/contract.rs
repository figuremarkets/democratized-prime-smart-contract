use crate::constants::{CONTRACT_NAME, CONTRACT_VERSION};
use crate::execute::update_asset_mappings::try_update_asset_mappings;
use crate::execute::update_asset_prices::try_update_asset_prices;
use crate::instantiate::instantiate_contract::instantiate_contract;
use crate::model::error::{invalid_funds, ContractError, QueryError};
use crate::msg::execute::ExecuteMsg;
use crate::msg::instantiate::InstantiateMsg;
use crate::msg::migrate::MigrateMsg;
use crate::query::prices::{query_prices_batch, query_prices_by_assets};
use crate::query::query_contract::query_state;
use crate::storage::contract_state::CONTRACT_STATE_V1;
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{ensure, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response};
use cw_ownable::get_ownership;
use democratized_prime_lib::common::migrate::migrate_contract;
use democratized_prime_lib::common::{update_ownership, LegacyMigration};
use democratized_prime_lib::price_oracle::model::ContractStateV1;
use democratized_prime_lib::price_oracle::msg::query::QueryMsg;

/// Entry point to instantiate the smart contract. Sets up the initial state and configurations
///
/// # Arguments
/// * `deps` - Contains access to blockchain tools, storage, and querying
/// * `env` - Information about the current block
/// * `info` - Information about the msg (sender/funds)
/// * `msg` - Input msg parameters passed during instantiation
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    instantiate_contract(deps, env.to_owned(), info.to_owned(), msg)
}

/// Entry point to execute the Price Oracle smart contract. Where the business logic of the contract lives.
///
/// # Arguments
/// * `deps` - Contains access to blockchain tools, storage, and querying
/// * `env` - Information about the current block
/// * `info` - Information about the msg (sender/funds)
/// * `msg` - Input msg parameters passed during execution. Supports multiple "types" which map to different actions/business logic.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    // all actions on this contract are admin routes that should not accept funds
    // note: if this ever changes, ensure the appropriate entrypoints have the no funds check
    ensure!(
        info.funds.is_empty(),
        invalid_funds("No funds accepted for price oracle actions")
    );
    match msg {
        ExecuteMsg::UpdateOwnership(action) => update_ownership(deps, env, info, action),
        ExecuteMsg::UpdateAssetPrices { prices } => {
            try_update_asset_prices(deps, env, info, prices)
        }
        ExecuteMsg::UpdateAssetMappings {
            to_update,
            to_remove,
        } => try_update_asset_mappings(deps, env, info, to_update, to_remove),
    }
}

/// Entry point to query the Price Oracle smart contract. Look up data stored in the contract.
///
/// # Arguments
/// * `deps` - Contains access to blockchain tools, storage, and querying
/// * `_env` - Information about the current block
/// * `msg` - Input msg parameters passed during query. Supports multiple "types" which map to different query logic.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, QueryError> {
    match msg {
        QueryMsg::Ownership {} => Ok(to_json_binary(&get_ownership(deps.storage)?)?),
        QueryMsg::GetState {} => query_state(deps.storage),
        QueryMsg::GetPrices { prev_asset, limit } => {
            query_prices_batch(deps.storage, prev_asset, limit)
        }
        QueryMsg::GetPricesByAsset { assets } => query_prices_by_assets(deps.storage, assets),
    }
}

/// Entry point to migrate the Price Oracle smart contract. Logic to move to a new code_id/wasm
///
/// # Arguments
/// * `deps` - Contains access to blockchain tools, storage, and querying
/// * `_env` - Information about the current block
/// * `_msg` - Input msg parameters passed during migration.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    migrate_contract::<ContractStateV1>(
        deps.storage,
        CONTRACT_NAME,
        CONTRACT_VERSION,
        Some(LegacyMigration {
            item: &CONTRACT_STATE_V1,
            api: deps.api,
        }),
    )
}
