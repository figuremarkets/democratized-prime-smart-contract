use cosmwasm_std::{Addr, Decimal256, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::collateral::CollateralAssetV1;
use crate::model::error::{illegal_state, ContractError};
use crate::model::{Denom, RateParamsV1};

/// How bad-debt liquidation (residual scaled debt after collateral exhausted) hits suppliers.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadDebtLossAllocation {
    /// Book `deficit_underlying`; reduce via **EliminateDeficit** or **SocializeDeficit**
    /// (pro-rata `liquidity_index` haircut + lower deficit).
    #[default]
    DeferredToDeficit,
    /// In the same liquidation tx, apply a pro-rata `liquidity_index` haircut for the bad-debt amount
    /// (no `deficit_underlying` increment for that slice).
    ImmediateLiquidityIndexHaircut,
}

impl BadDebtLossAllocation {
    /// Stable snake_case tag for event attributes (matches JSON).
    pub fn as_str(self) -> &'static str {
        match self {
            BadDebtLossAllocation::DeferredToDeficit => "deferred_to_deficit",
            BadDebtLossAllocation::ImmediateLiquidityIndexHaircut => {
                "immediate_liquidity_index_haircut"
            }
        }
    }
}

/// Operational state of the pool. Controls which actions are allowed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum OperationalState {
    /// Normal operation: all actions allowed.
    #[default]
    Active,
    /// New lend and borrow disabled; withdraw, repay, collateral, transfer and owner-only actions allowed.
    Frozen,
    /// Full freeze: only owner config allowed (including [`crate::msg::ExecuteMsg::UpdateOwnership`]). [`crate::msg::ExecuteMsg::Liquidate`], [`crate::msg::ExecuteMsg::Repay`],
    /// [`crate::msg::ExecuteMsg::Withdraw`], [`crate::msg::ExecuteMsg::WithdrawReserve`],
    /// [`crate::msg::ExecuteMsg::AddCollateral`], and [`crate::msg::ExecuteMsg::RemoveCollateral`] are blocked —
    /// no funds/collateral in or out, no liquidations.
    Paused,
}

/// Contract config (immutable after instantiation except collateral list). Owner is stored by [`cw_ownable`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ContractStateV1 {
    #[serde(rename = "c_n")]
    pub contract_name: String,
    #[serde(rename = "d")]
    pub description: String,
    /// CW20 contract address for the repo token (receipt token for supplied liquidity).
    /// **`Some` immediately** when the pool was instantiated with **`RepoTokenConfig::Existing`**.
    /// **`None` until `reply`** when using **`RepoTokenConfig::New`**, then set after the repo token `SubMsg` succeeds in the same transaction.
    #[serde(default, rename = "atca")]
    pub repo_token_cw20_address: Option<Addr>,
    #[serde(rename = "ld")]
    pub lending_denom: Denom,
    #[serde(rename = "rp")]
    pub rate_params: RateParamsV1,
    #[serde(rename = "lra")]
    pub lender_required_attrs: Vec<String>,
    #[serde(rename = "bra")]
    pub borrower_required_attrs: Vec<String>,
    #[serde(rename = "poa")]
    pub price_oracle_address: Addr,
    pub max_borrower_collateral_types: u32,
    /// LTV must stay below this to borrow or remove collateral.
    #[serde(rename = "mr")]
    pub margin_rate: Decimal256,
    /// LTV at or above this allows liquidation.
    #[serde(rename = "lr")]
    pub liquidation_rate: Decimal256,
    /// Liquidation bonus: collateral seized must be worth at most (repay value × this rate). E.g. 1.02 = 2% bonus.
    #[serde(rename = "lbr")]
    pub liquidation_bonus_rate: Decimal256,
    /// Minimum amount to lend (in lending denom base units). Avoids dust and rounding issues.
    #[serde(rename = "min_lend")]
    pub min_lend: Uint128,
    /// Minimum amount per borrow (in lending denom base units). Independent of [`ContractStateV1::min_lend`].
    #[serde(rename = "min_borrow")]
    pub min_borrow: Uint128,

    /// Supported collateral assets (asset_id and optional haircut). The contract owner can update via [`crate::msg::ExecuteMsg::UpdateSupportedCollateral`].
    #[serde(rename = "sca", default)]
    pub supported_collateral_assets: Vec<CollateralAssetV1>,

    /// Operational state: Active (all allowed), Frozen (Lend/Borrow blocked; Receive for withdraw/transfer allowed), Paused (contract-owner config only).
    #[serde(rename = "op", default)]
    pub operational_state: OperationalState,

    /// When set, withdraw with commit_funds: true will emit MsgCommitFundsRequest to this Provenance exchange market. None = no on-chain commit.
    #[serde(rename = "commit_market_id", default)]
    pub commit_market_id: Option<u32>,

    /// Bad-debt handling at liquidation: defer to `deficit_underlying`, or immediate supplier index haircut.
    #[serde(rename = "bdla", default)]
    pub bad_debt_loss_allocation: BadDebtLossAllocation,
}

impl ContractStateV1 {
    /// Repo token CW20 address after instantiate + reply have completed.
    pub fn repo_token_addr(&self) -> Result<Addr, ContractError> {
        self.repo_token_cw20_address
            .clone()
            .ok_or_else(|| illegal_state("repo token not bound"))
    }
}
