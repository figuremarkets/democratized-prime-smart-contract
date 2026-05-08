use crate::constants::{
    ATTRIBUTE_ACTION_NAME, CONTRACT_NAME, CONTRACT_VERSION, MAX_LENDER_BORROWER_REQUIRED_ATTRS,
    REPO_TOKEN_INSTANTIATE_REPLY_ID,
};
use crate::model::error::{illegal_argument, ContractError};
use crate::model::{ContractStateV1, OperationalState, ReserveStateV1};
use crate::msg::instantiate::{InstantiateMsg, RepoTokenConfig};
use crate::storage::{set_contract_state_v1, set_reserve_state_v1};
use cosmwasm_std::{
    ensure, to_json_binary, CosmosMsg, Decimal256, DepsMut, Env, MessageInfo, Response, SubMsg,
    WasmMsg,
};
use cw2::set_contract_version;
use cw_ownable::initialize_owner;
use democratized_prime_lib::repo_token::{self as repo_token_msg, validate_repo_token_meta};
use result_extensions::ResultExtensions;
use std::collections::HashSet;

pub const ACTION: &str = "instantiate";

/// Initialize the pool: contract state, reserve (indexes = 1), and supported collateral config.
/// **Repo token:** [`RepoTokenConfig::Existing`] validates and stores the address immediately.
/// [`RepoTokenConfig::New`] stores state with no repo address yet, then a `SubMsg` instantiates the CW20; `reply` binds the address.
/// The SubMsg payload is [`democratized_prime_lib::repo_token::InstantiateMsg`]; name/symbol/decimals use [`democratized_prime_lib::repo_token::validate_repo_token_meta`], shared with `repo_token_cw20`’s `instantiate`.
/// The `reply` callback runs in the context of **this** pool contract (the submessage sender), not an EOA.
pub fn instantiate_contract(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    msg.lending_denom.validate()?;
    msg.rate_params.validate()?;
    let mut seen_ids = HashSet::new();
    for asset in &msg.supported_collateral_assets {
        asset.validate()?;
        ensure!(
            asset.asset_id != msg.lending_denom.name,
            illegal_argument(format!(
                "Collateral asset cannot be the lending denom ({})",
                asset.asset_id
            ))
        );
        ensure!(
            seen_ids.insert(asset.asset_id.clone()),
            illegal_argument(format!(
                "Duplicate supported_collateral_assets entry for asset_id: {}",
                asset.asset_id
            ))
        );
    }

    let (repo_token_cw20_address, new_repo_token_instantiate) = match &msg.repo_token {
        RepoTokenConfig::Existing {
            repo_token_cw20_contract_address,
        } => {
            ensure!(
                !repo_token_cw20_contract_address.trim().is_empty(),
                illegal_argument("repo_token_cw20_contract_address cannot be empty")
            );
            let addr = deps
                .api
                .addr_validate(repo_token_cw20_contract_address.trim())?;
            (Some(addr), None)
        }
        RepoTokenConfig::New {
            repo_token_code_id,
            repo_token_name,
            repo_token_symbol,
            repo_token_decimals,
        } => {
            ensure!(
                *repo_token_code_id > 0,
                illegal_argument("repo_token_code_id must be greater than zero")
            );
            validate_repo_token_meta(
                repo_token_name.trim(),
                repo_token_symbol.trim(),
                *repo_token_decimals,
            )
            .map_err(|m| illegal_argument(format!("repo_token: {m}")))?;
            (
                None,
                Some((
                    *repo_token_code_id,
                    repo_token_name.trim().to_string(),
                    repo_token_symbol.trim().to_string(),
                    *repo_token_decimals,
                )),
            )
        }
    };

    ensure!(
        !msg.contract_name.trim().is_empty(),
        illegal_argument("contract_name cannot be empty")
    );
    ensure!(
        !msg.price_oracle_address.trim().is_empty(),
        illegal_argument("price_oracle_address cannot be empty")
    );
    let price_oracle_address = deps.api.addr_validate(msg.price_oracle_address.trim())?;
    ensure!(
        !msg.min_lend.is_zero(),
        illegal_argument("min_lend must be at least 1")
    );
    ensure!(
        !msg.min_borrow.is_zero(),
        illegal_argument("min_borrow must be at least 1")
    );
    ensure!(
        !msg.margin_rate.is_zero(),
        illegal_argument("margin_rate must be greater than zero")
    );
    ensure!(
        !msg.liquidation_rate.is_zero(),
        illegal_argument("liquidation_rate must be greater than zero")
    );
    ensure!(
        msg.liquidation_rate <= Decimal256::one(),
        illegal_argument("liquidation_rate must be less than or equal to 1")
    );
    ensure!(
        msg.margin_rate < msg.liquidation_rate,
        illegal_argument("margin_rate must be less than liquidation_rate")
    );
    ensure!(
        msg.liquidation_bonus_rate > Decimal256::one(),
        illegal_argument("liquidation_bonus_rate must be greater than 1 (e.g. 1.02 for 2%)")
    );
    let bonus_times_margin = msg
        .liquidation_bonus_rate
        .checked_mul(msg.margin_rate)
        .map_err(|_| illegal_argument("liquidation_bonus_rate * margin_rate overflow"))?;
    ensure!(
        bonus_times_margin < Decimal256::one(),
        illegal_argument(
            "liquidation_bonus_rate * margin_rate must be < 1 (otherwise 1 - bonus*margin_rate \
             underflows and liquidations are impossible; e.g. bonus=1.02 and margin_rate=0.99 is invalid)"
        )
    );
    ensure!(
        msg.max_borrower_collateral_types > 0,
        illegal_argument("max_borrower_collateral_types must be at least 1")
    );
    ensure!(
        msg.lender_required_attrs.len() <= MAX_LENDER_BORROWER_REQUIRED_ATTRS,
        illegal_argument(format!(
            "No more than [{}] lender required attributes allowed",
            MAX_LENDER_BORROWER_REQUIRED_ATTRS
        ))
    );
    ensure!(
        msg.borrower_required_attrs.len() <= MAX_LENDER_BORROWER_REQUIRED_ATTRS,
        illegal_argument(format!(
            "No more than [{}] borrower required attributes allowed",
            MAX_LENDER_BORROWER_REQUIRED_ATTRS
        ))
    );

    let pool = env.contract.address.clone();

    let contract_state = ContractStateV1 {
        contract_name: msg.contract_name.clone(),
        description: msg.description,
        repo_token_cw20_address,
        lending_denom: msg.lending_denom.clone(),
        rate_params: msg.rate_params.clone(),
        lender_required_attrs: msg.lender_required_attrs,
        borrower_required_attrs: msg.borrower_required_attrs,
        price_oracle_address,
        max_borrower_collateral_types: msg.max_borrower_collateral_types,
        margin_rate: msg.margin_rate,
        liquidation_rate: msg.liquidation_rate,
        liquidation_bonus_rate: msg.liquidation_bonus_rate,
        min_lend: msg.min_lend,
        min_borrow: msg.min_borrow,
        supported_collateral_assets: msg.supported_collateral_assets,
        operational_state: OperationalState::Active,
        commit_market_id: msg.commit_market_id,
        bad_debt_loss_allocation: msg.bad_debt_loss_allocation,
    };
    set_contract_state_v1(deps.storage, &contract_state)?;
    initialize_owner(deps.storage, deps.api, Some(info.sender.as_str()))?;

    let reserve = ReserveStateV1 {
        liquidity_index: Decimal256::one(),
        borrow_index: Decimal256::one(),
        last_updated_at: env.block.time,
        total_scaled_liquidity: 0,
        total_scaled_borrow: 0,
        accrued_reserve: 0,
        deficit_underlying: 0,
    };
    set_reserve_state_v1(deps.storage, &reserve)?;

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let mut response = Response::new().add_attribute(ATTRIBUTE_ACTION_NAME, ACTION);

    if let Some((repo_token_code_id, repo_token_name, repo_token_symbol, repo_token_decimals)) =
        new_repo_token_instantiate
    {
        let owner_address = info.sender.to_string();
        let pool_address = pool.to_string();
        let repo_instantiate = repo_token_msg::InstantiateMsg {
            name: repo_token_name,
            symbol: repo_token_symbol,
            decimals: repo_token_decimals,
            owner: owner_address.clone(),
            minter: pool_address.clone(),
            pool_address: Some(pool_address),
        };

        let instantiate_wasm = WasmMsg::Instantiate {
            admin: Some(owner_address),
            code_id: repo_token_code_id,
            msg: to_json_binary(&repo_instantiate)?,
            funds: vec![],
            label: format!("{}-repo-token", msg.contract_name),
        };

        // `reply_always` so `reply` runs on failure and can return Err (atomic rollback).
        response = response.add_submessage(SubMsg::reply_always(
            CosmosMsg::Wasm(instantiate_wasm),
            REPO_TOKEN_INSTANTIATE_REPLY_ID,
        ));
    }

    response.to_ok()
}
