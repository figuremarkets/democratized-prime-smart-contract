use crate::model::{BadDebtLossAllocation, CollateralAssetV1, Denom, RateParamsV1};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal256, Uint128};

/// How the pool binds to the repo (receipt) CW20: use an existing contract or instantiate a new one in the same transaction.
#[cw_serde]
pub enum RepoTokenConfig {
    /// Use a repo token contract that is already deployed (e.g. shared or pre-created). Address is validated and stored on instantiate; no `SubMsg` / `reply`.
    Existing {
        repo_token_cw20_contract_address: String,
    },
    /// Instantiate a new `repo_token_cw20` from this code id; the pool emits `WasmMsg::Instantiate` as a `SubMsg` and binds the address in `reply`.
    /// The SubMsg uses [`democratized_prime_lib::repo_token::InstantiateMsg`]; name, symbol, and decimals are checked with [`democratized_prime_lib::repo_token::validate_repo_token_meta`] (same as `repo_token_cw20`’s `instantiate`).
    New {
        /// Code ID of the `repo_token_cw20` contract.
        repo_token_code_id: u64,
        /// CW20 name (validated with `democratized_prime_lib::repo_token::validate_repo_token_meta`).
        repo_token_name: String,
        /// CW20 symbol (same validation).
        repo_token_symbol: String,
        /// CW20 decimals (same validation; typically same as lending denom).
        repo_token_decimals: u8,
    },
}

#[cw_serde]
pub struct InstantiateMsg {
    pub contract_name: String,
    pub description: String,
    pub repo_token: RepoTokenConfig,
    pub lending_denom: Denom,
    pub rate_params: RateParamsV1,
    /// Attribute names for lending: sender must have all of these (Provenance attributes).
    /// - Empty list = no attribute required.
    pub lender_required_attrs: Vec<String>,
    /// Attribute names for borrowing: sender must have all of these.
    /// - Empty list = no attribute required.
    pub borrower_required_attrs: Vec<String>,
    /// Price oracle contract address.
    /// - Must be a valid bech32 address.
    pub price_oracle_address: String,
    pub max_borrower_collateral_types: u32,
    pub margin_rate: Decimal256,
    pub liquidation_rate: Decimal256,
    /// Liquidation bonus rate: max collateral value = repay value × this (e.g. 1.02 = 2% bonus).
    /// - Must be > 1.
    pub liquidation_bonus_rate: Decimal256,
    /// Minimum amount per lend (in lending denom base units).
    /// - Must be at least 1.
    /// - Used to avoid dust and zero-size lends.
    pub min_lend: Uint128,
    /// Minimum amount per borrow (in lending denom base units).
    /// - Must be at least 1.
    /// - Independent of [`InstantiateMsg::min_lend`].
    pub min_borrow: Uint128,
    pub supported_collateral_assets: Vec<CollateralAssetV1>,
    /// When set, withdraw with commit_funds: true will emit MsgCommitFundsRequest to this Provenance exchange market. None = no on-chain commit.
    #[serde(default)]
    pub commit_market_id: Option<u32>,
    /// How bad-debt liquidation affects suppliers (see [`crate::model::BadDebtLossAllocation`]). Default: defer to `deficit_underlying`.
    #[serde(default)]
    pub bad_debt_loss_allocation: BadDebtLossAllocation,
}
