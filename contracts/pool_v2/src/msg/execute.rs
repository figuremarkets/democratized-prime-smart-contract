use crate::model::{BadDebtLossAllocation, CollateralAssetV1, OperationalState, RateParamsV1};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal256, Uint128};
use cw20::Cw20ReceiveMsg;
use cw_ownable::cw_ownable_execute;
use std::collections::BTreeMap;

/// How a partial `deficit_underlying` reduction is funded (one mode per call).
#[cw_serde]
pub enum EliminateDeficitFunding {
    /// Contract owner only: use up to `max_underlying` from `accrued_reserve` (no native funds on the message).
    AccruedReserve { max_underlying: Uint128 },
    /// Any sender: attach lending-denom coins; apply up to `min(max_underlying, deficit, sent)`; excess refunded to sender.
    Bank { max_underlying: Uint128 },
}

/// Payload for CW20 Send to this contract. Use with the repo token CW20 Send message.
#[cw_serde]
pub enum Cw20ReceivePayload {
    /// Withdraw underlying amount; send at least the required scaled repo token. Excess is refunded.
    /// When this lender has per-address "require commit on exit" set, pass commit_funds: true.
    Withdraw {
        amount: Uint128,
        /// When this lender has per-address "require commit on exit" set, pass true.
        #[serde(default)]
        commit_funds: Option<bool>,
    },
    /// Withdraw: burn all sent repo token and receive equivalent underlying.
    /// When this lender has per-address "require commit on exit" set, pass commit_funds: true.
    WithdrawExact {
        #[serde(default)]
        commit_funds: Option<bool>,
    },
    /// Transfer repo token (underlying amount) to recipient; send at least required scaled. Excess refunded.
    /// Not allowed when this lender has "require commit on exit" set; they must withdraw with commit_funds first.
    Transfer { recipient: String, amount: Uint128 },
    /// Transfer all sent repo token to recipient.
    /// Not allowed when this lender has "require commit on exit" set; they must withdraw with commit_funds first.
    TransferExact { recipient: String },
}

#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg {
    /// Lend assets to the pool; sender receives repo token (CW20) balance.
    Lend {},

    /// Called by the repo CW20 contract when a user Sends tokens to this pool. Payload specifies Withdraw/Transfer.
    Receive(Cw20ReceiveMsg),

    /// Borrow against collateral.
    Borrow { amount: Uint128 },

    /// Repay borrowed amount (from info.funds).
    Repay {},

    /// Add collateral (from info.funds).
    AddCollateral {},

    /// Remove collateral.
    RemoveCollateral {
        to_remove: BTreeMap<String, Uint128>,
    },

    /// Liquidate a borrower (contract owner only). Liquidator repays debt via funds (one coin, lending
    /// denom); repay amount = min(sent, debt), excess refunded. Seized collateral value must be
    /// in [100%, liquidation_bonus_rate] of the amount repaid.
    Liquidate {
        borrower: String,
        /// Asset id -> amount to seize from the borrower. Market value (price × amount) must be in [100%, liquidation_bonus_rate] of amount repaid.
        collateral_to_seize: BTreeMap<String, Uint128>,
    },

    /// Update supported collateral assets (contract owner).
    UpdateSupportedCollateral {
        to_update: Vec<CollateralAssetV1>,
        to_remove: Vec<String>,
    },

    /// Withdraw accrued protocol reserve (contract owner only; no funds). Sends full accrued_reserve in lending denom to recipient, or to the contract owner if recipient is None.
    WithdrawReserve {
        /// Address to receive the reserve; if None, sends to the contract owner.
        recipient: Option<String>,
    },

    /// Reduce `deficit_underlying`. **`accrued_reserve`:** contract owner only. **`bank`:** any account may send lending coins. See `EliminateDeficitFunding`; partial clearance allowed.
    EliminateDeficit { funding: EliminateDeficitFunding },

    /// Pro-rata `liquidity_index` haircut on `min(max_amount, deficit_underlying)` (contract owner, no funds). Fails if applied loss is not strictly below `total_liquidity` (same as immediate bad-debt haircut).
    SocializeDeficit { max_amount: Uint128 },

    /// Set operational state (contract owner only). Active / Frozen / Paused.
    SetOperationalState { state: OperationalState },

    /// Set required lender attributes (contract owner only). Sender must have all of these to Lend or receive Transfer. Empty list = no check.
    SetLenderRequiredAttrs { lender_required_attrs: Vec<String> },

    /// Set required borrower attributes (contract owner only). Sender must have all of these to Borrow, AddCollateral, or RemoveCollateral. Empty list = no check.
    SetBorrowerRequiredAttrs {
        borrower_required_attrs: Vec<String>,
    },

    /// Update contract config (contract owner only). Only provided (non-null) fields are updated. After apply, margin_rate < liquidation_rate and liquidation_bonus_rate * margin_rate < 1.
    UpdateContractConfig {
        margin_rate: Option<Decimal256>,
        liquidation_rate: Option<Decimal256>,
        liquidation_bonus_rate: Option<Decimal256>,
        price_oracle_address: Option<String>,
        min_lend: Option<Uint128>,
        min_borrow: Option<Uint128>,
        max_borrower_collateral_types: Option<u32>,
        /// When set: set Provenance **commit** market id. Omitted (or JSON `null`) = no change to this field.
        #[serde(default)]
        commit_market_id: Option<u32>,
        /// When set: updates [`crate::model::BadDebtLossAllocation`] (how bad-debt liquidation hits suppliers).
        /// Changing the value is rejected while **`deficit_underlying`** on the reserve is positive.
        #[serde(default)]
        bad_debt_loss_allocation: Option<BadDebtLossAllocation>,
    },

    /// Update interest rate params (contract owner only). Full replacement; validated same as at instantiate.
    UpdateRateParams { rate_params: RateParamsV1 },

    /// Per-address "require commit on exit" for lenders (contract owner only). When set to true for an address,
    /// that lender must pass commit_funds: true in Withdraw/WithdrawExact payloads. None = remove override.
    SetLenderRequireCommitOnExit {
        address: String,
        require: Option<bool>,
    },

    /// Withdraw a lender's supply on their behalf (contract owner only). Burns the lender's repo token and sends underlying to the lender. Use when closing a pool.
    /// Does not check require_commit_on_exit; optionally pass commit_funds: true to emit MsgCommitFundsRequest when commit_market_id is set.
    Withdraw {
        /// Lender whose supply to withdraw; they receive the underlying.
        lender: String,
        /// Max underlying amount to withdraw. If None, withdraws full lent supply.
        amount: Option<Uint128>,
        /// When true and commit_market_id is set, emit MsgCommitFundsRequest for the withdrawn amount to the lender's account. Omitted/None = no commit.
        #[serde(default)]
        commit_funds: Option<bool>,
    },
}
