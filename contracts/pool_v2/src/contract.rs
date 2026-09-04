use crate::constants::{ATTRIBUTE_CUSTODIAN, CONTRACT_NAME, CONTRACT_VERSION};
use crate::execute::{
    add_collateral, borrow, eliminate_deficit, execute_withdraw, lend, liquidate, receive,
    remove_collateral, repay, set_borrower_required_attrs, set_lender_require_commit_on_exit,
    set_lender_required_attrs, set_operational_state, socialize_deficit, update_contract_config,
    update_rate_params, update_supported_collateral, withdraw_reserve, UpdateContractConfigParams,
};
use crate::instantiate::{instantiate_contract, reply as reply_handler};
use crate::model::error::{illegal_argument, illegal_state, ContractError, QueryError};
use crate::model::{ContractStateV1, OperationalState};
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::query::{
    query_borrower_position, query_collateral_requirements, query_lender_status, query_reserve,
    query_state,
};
use crate::storage::contract_state::ITEM;
use crate::storage::{get_contract_state_v1, set_contract_state_v1};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    ensure, to_json_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Reply, Response,
};
use cw_ownable::get_ownership;
use democratized_prime_lib::common::{migrate_contract, update_ownership, LegacyMigration};

pub const ASSERT_CUSTODIAN_ERR: &str =
    "Custodian account must be set on contract state going forward";

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    instantiate_contract(deps, env, info, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> Result<Response, ContractError> {
    reply_handler(deps, env, msg)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let contract = get_contract_state_v1(deps.storage)?;
    // When Paused (e.g. emergency/bug): full freeze. Only custodian config changes
    // are allowed, with the exception of UpdateOwnership (owner only); no funds/collateral
    // in or out. Liquidate is blocked; WithdrawReserve is also blocked (it sends funds).
    let allowed_when_paused = matches!(
        &msg,
        ExecuteMsg::UpdateSupportedCollateral { .. }
            | ExecuteMsg::SetOperationalState { .. }
            | ExecuteMsg::UpdateOwnership(_)
            | ExecuteMsg::SetLenderRequiredAttrs { .. }
            | ExecuteMsg::SetBorrowerRequiredAttrs { .. }
            | ExecuteMsg::SetLenderRequireCommitOnExit { .. }
            | ExecuteMsg::UpdateContractConfig { .. }
            | ExecuteMsg::UpdateRateParams { .. }
    );
    ensure!(
        allowed_when_paused || contract.operational_state != OperationalState::Paused,
        illegal_state(
            "Contract is paused; only custodian config updates and ownership transfer is allowed"
        )
    );
    ensure!(
        contract.operational_state != OperationalState::Frozen
            || !matches!(&msg, ExecuteMsg::Lend { .. } | ExecuteMsg::Borrow { .. }),
        illegal_state("Contract is frozen; lend and borrow are disabled")
    );

    match msg {
        ExecuteMsg::UpdateOwnership(action) => update_ownership(deps, env, info, action),
        ExecuteMsg::Lend {} => lend(deps, env, info),
        ExecuteMsg::Receive(cw20_msg) => receive(deps, env, info, cw20_msg),
        ExecuteMsg::Borrow { amount } => borrow(deps, env, info, amount),
        ExecuteMsg::Repay {} => repay(deps, env, info),
        ExecuteMsg::AddCollateral {} => add_collateral(deps, env, info),
        ExecuteMsg::RemoveCollateral { to_remove } => {
            remove_collateral(deps, env, info, &to_remove)
        }
        ExecuteMsg::Liquidate {
            borrower,
            collateral_to_seize,
        } => liquidate(deps, env, info, borrower, &collateral_to_seize),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        } => update_supported_collateral(deps, env, info, &to_update, &to_remove),
        ExecuteMsg::WithdrawReserve { recipient } => {
            withdraw_reserve::withdraw_reserve(deps, env, info, recipient)
        }
        ExecuteMsg::EliminateDeficit { funding } => eliminate_deficit(deps, env, info, funding),
        ExecuteMsg::SocializeDeficit { max_amount } => {
            socialize_deficit(deps, env, info, max_amount)
        }
        ExecuteMsg::SetOperationalState { state } => set_operational_state(deps, env, info, state),
        ExecuteMsg::SetLenderRequiredAttrs {
            lender_required_attrs,
        } => set_lender_required_attrs(deps, env, info, lender_required_attrs),
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs,
        } => set_borrower_required_attrs(deps, env, info, borrower_required_attrs),
        ExecuteMsg::UpdateContractConfig {
            margin_rate,
            liquidation_rate,
            liquidation_bonus_rate,
            price_oracle_address,
            min_lend,
            min_borrow,
            max_borrower_collateral_types,
            commit_market_id,
            bad_debt_loss_allocation,
            custodian,
            max_liquidation_staleness_seconds,
        } => update_contract_config(
            deps,
            env,
            info,
            UpdateContractConfigParams {
                margin_rate,
                liquidation_rate,
                liquidation_bonus_rate,
                price_oracle_address,
                min_lend,
                min_borrow,
                max_borrower_collateral_types,
                commit_market_id,
                bad_debt_loss_allocation,
                custodian,
                max_liquidation_staleness_seconds,
            },
        ),
        ExecuteMsg::UpdateRateParams { rate_params } => {
            update_rate_params(deps, env, info, rate_params)
        }
        ExecuteMsg::SetLenderRequireCommitOnExit { address, require } => {
            set_lender_require_commit_on_exit(deps, env, info, address, require)
        }
        ExecuteMsg::Withdraw {
            lender,
            amount,
            commit_funds,
        } => execute_withdraw(deps, env, info, lender, amount, commit_funds),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> Result<Binary, QueryError> {
    match msg {
        QueryMsg::Ownership {} => Ok(to_json_binary(&get_ownership(deps.storage)?)?),
        QueryMsg::GetState {} => query_state(deps, env),
        QueryMsg::GetReserve {} => query_reserve(deps, env),
        QueryMsg::GetBorrowerPosition { address } => query_borrower_position(deps, env, &address),
        QueryMsg::GetCollateralRequirements {
            borrower,
            new_loan_amount,
            collateral_assets,
        } => query_collateral_requirements(
            deps,
            env,
            borrower.as_deref(),
            new_loan_amount,
            &collateral_assets,
        ),
        QueryMsg::GetLenderStatus { address } => query_lender_status(deps, &address),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    // Check the contract's existing custodian account and enforce one being set if not already done:
    let custodian_account: Option<Addr> = {
        let contract_state: ContractStateV1 = get_contract_state_v1(deps.storage)?;
        match (&contract_state.custodian, &msg.custodian) {
            // no existing custodian set on the contract and no update provided; return an error
            (None, None) => return Err(illegal_argument(ASSERT_CUSTODIAN_ERR)),
            // contract custodian will be updated to use the given account address:
            (_, Some(addr)) => Some(deps.api.addr_validate(addr.trim())?),
            // contract has an existing custodian, but no update is given; do nothing
            (Some(_), None) => None,
        }
    };

    let mut response: Response = migrate_contract::<ContractStateV1>(
        deps.storage,
        CONTRACT_NAME,
        CONTRACT_VERSION,
        Some(LegacyMigration {
            item: &ITEM,
            api: deps.api,
        }),
    )?;

    if let Some(custodian_account) = custodian_account {
        // Get the updated state:
        let mut contract_state: ContractStateV1 = get_contract_state_v1(deps.storage)?;
        contract_state.custodian = Some(custodian_account.to_owned());
        set_contract_state_v1(deps.storage, &contract_state)?;
        response = response.add_attribute(ATTRIBUTE_CUSTODIAN, custodian_account.to_string());
    }

    Ok(response)
}
