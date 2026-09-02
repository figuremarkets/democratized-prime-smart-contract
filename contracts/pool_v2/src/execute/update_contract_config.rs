//! Contract owner only: update contract config (margin/liquidation rates, oracle, min amounts, max collateral types).
//! Only provided (non-null) fields are updated. Validates invariants after apply.
//!
//! **Liquidation rate:** To avoid forcibly liquidating positions that were previously safe,
//! `liquidation_rate` may only be **increased** (never decreased). Decreasing would lower the
//! LTV bar and could make many positions liquidatable at once.
//!
//! **`bad_debt_loss_allocation`:** May only be changed when **`deficit_underlying`** is zero.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_CONTRACT_STATE_JSON};
use crate::model::error::{illegal_argument, invalid_funds, ContractError};
use crate::model::{
    BadDebtLossAllocation, LiquidationAccess, MAX_ALLOWED_LIQUIDATION_STALENESS_SECONDS,
};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1, set_contract_state_v1};
use crate::utils::assert_custodian;
use cosmwasm_std::{ensure, Decimal256, DepsMut, Env, MessageInfo, Response, Uint128};
use result_extensions::ResultExtensions;

pub const ACTION: &str = "update_contract_config";
pub const ASSERT_CUSTODIAN_ERR: &str =
    "Only the contract custodian may update contract configuration";

/// Optional config fields for UpdateContractConfig. Only provided (non-null) fields are applied.
#[derive(Clone, Default)]
pub struct UpdateContractConfigParams {
    pub margin_rate: Option<Decimal256>,
    pub liquidation_rate: Option<Decimal256>,
    pub liquidation_bonus_rate: Option<Decimal256>,
    pub price_oracle_address: Option<String>,
    pub min_lend: Option<Uint128>,
    pub min_borrow: Option<Uint128>,
    pub max_borrower_collateral_types: Option<u32>,
    pub commit_market_id: Option<u32>,
    pub bad_debt_loss_allocation: Option<BadDebtLossAllocation>,
    pub custodian: Option<String>,
    pub max_liquidation_staleness_seconds: Option<u64>,
    pub liquidation_access: Option<LiquidationAccess>,
}

/// Update contract config. Contract custodian only; no funds. Only provided fields are updated.
/// After apply: margin_rate < liquidation_rate, liquidation_bonus_rate > 1, bonus * margin_rate < 1.
#[allow(clippy::too_many_arguments)]
pub fn update_contract_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    params: UpdateContractConfigParams,
) -> Result<Response, ContractError> {
    let mut contract = get_contract_state_v1(deps.storage)?;

    assert_custodian(&contract, &info.sender, ASSERT_CUSTODIAN_ERR)?;

    ensure!(info.funds.is_empty(), invalid_funds("No funds accepted"));

    let has_any = params.margin_rate.is_some()
        || params.liquidation_rate.is_some()
        || params.liquidation_bonus_rate.is_some()
        || params.price_oracle_address.is_some()
        || params.min_lend.is_some()
        || params.min_borrow.is_some()
        || params.max_borrower_collateral_types.is_some()
        || params.commit_market_id.is_some()
        || params.bad_debt_loss_allocation.is_some()
        || params.custodian.is_some()
        || params.max_liquidation_staleness_seconds.is_some()
        || params.liquidation_access.is_some();
    ensure!(
        has_any,
        illegal_argument("At least one config field must be provided")
    );

    if let Some(v) = params.margin_rate {
        contract.margin_rate = v;
    }
    if let Some(v) = params.liquidation_rate {
        ensure!(
            v >= contract.liquidation_rate,
            illegal_argument(
                "liquidation_rate may only be increased (not decreased) to avoid making \
                 previously safe positions liquidatable"
            )
        );
        contract.liquidation_rate = v;
    }
    if let Some(v) = params.liquidation_bonus_rate {
        contract.liquidation_bonus_rate = v;
    }
    if let Some(s) = params.price_oracle_address {
        ensure!(
            !s.trim().is_empty(),
            illegal_argument("price_oracle_address cannot be empty")
        );
        contract.price_oracle_address = deps.api.addr_validate(s.trim())?;
    }
    if let Some(v) = params.min_lend {
        ensure!(
            !v.is_zero(),
            illegal_argument("min_lend must be at least 1")
        );
        contract.min_lend = v;
    }
    if let Some(v) = params.min_borrow {
        ensure!(
            !v.is_zero(),
            illegal_argument("min_borrow must be at least 1")
        );
        contract.min_borrow = v;
    }
    if let Some(v) = params.max_borrower_collateral_types {
        ensure!(
            v > 0,
            illegal_argument("max_borrower_collateral_types must be at least 1")
        );
        contract.max_borrower_collateral_types = v;
    }
    if let Some(market_id) = params.commit_market_id {
        contract.commit_market_id = Some(market_id);
    }
    if let Some(v) = params.bad_debt_loss_allocation {
        if v != contract.bad_debt_loss_allocation {
            let reserve = get_reserve_state_v1(deps.storage)?;
            ensure!(
                reserve.deficit_underlying == 0,
                illegal_argument(
                    "cannot change bad_debt_loss_allocation while deficit_underlying > 0; \
                     clear the deficit with EliminateDeficit or SocializeDeficit first"
                )
            );
        }
        contract.bad_debt_loss_allocation = v;
    }
    if let Some(new_custodian) = params.custodian {
        let new_custodian = deps.api.addr_validate(new_custodian.trim())?;
        contract.custodian = Some(new_custodian);
    }
    if let Some(v) = params.max_liquidation_staleness_seconds {
        contract.max_liquidation_staleness_seconds = v;
    }
    if let Some(v) = params.liquidation_access {
        contract.liquidation_access = v;
    }

    ensure!(
        !contract.margin_rate.is_zero(),
        illegal_argument("margin_rate must be greater than zero")
    );
    ensure!(
        !contract.liquidation_rate.is_zero(),
        illegal_argument("liquidation_rate must be greater than zero")
    );
    ensure!(
        contract.liquidation_rate <= Decimal256::one(),
        illegal_argument("liquidation_rate must be less than or equal to 1")
    );
    ensure!(
        contract.margin_rate < contract.liquidation_rate,
        illegal_argument("margin_rate must be less than liquidation_rate")
    );
    ensure!(
        contract.liquidation_bonus_rate > Decimal256::one(),
        illegal_argument("liquidation_bonus_rate must be greater than 1 (e.g. 1.02 for 2%)")
    );
    let bonus_times_margin = contract
        .liquidation_bonus_rate
        .checked_mul(contract.margin_rate)
        .map_err(|_| illegal_argument("liquidation_bonus_rate * margin_rate overflow"))?;
    ensure!(
        bonus_times_margin < Decimal256::one(),
        illegal_argument(
            "liquidation_bonus_rate * margin_rate must be < 1 (otherwise liquidations are impossible)"
        )
    );
    ensure!(
        contract.max_liquidation_staleness_seconds <= MAX_ALLOWED_LIQUIDATION_STALENESS_SECONDS,
        illegal_argument("max_liquidation_staleness_seconds exceeds maximum")
    );

    set_contract_state_v1(deps.storage, &contract)?;
    Response::new()
        .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
        .add_attribute(
            ATTRIBUTE_CONTRACT_STATE_JSON,
            serde_json::to_string(&contract).unwrap_or_default(),
        )
        .to_ok()
}
