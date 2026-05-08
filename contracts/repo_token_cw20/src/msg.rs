use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Uint128};
use cw20::{AllAccountsResponse, BalanceResponse, MinterResponse, TokenInfoResponse};
use cw_ownable::{cw_ownable_execute, cw_ownable_query};

/// Shared with **`pool_v2`** (`WasmMsg::Instantiate` for `repo_token.new`): same JSON as `democratized_prime_lib::repo_token::InstantiateMsg`.
pub use democratized_prime_lib::repo_token::InstantiateMsg;

#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg {
    /// Standard CW20. Only minter (pool) may mint.
    Mint { recipient: String, amount: Uint128 },
    /// Burn from caller (minter). Used when pool has received repo token via Send (user withdraw) and burns from its own balance.
    Burn { amount: Uint128 },
    /// Burn from another address. Minter (pool) only; used when admin withdraws a lender's supply on their behalf (no Send to pool).
    BurnFrom { owner: String, amount: Uint128 },
    /// CW20 Transfer (scaled units). Only the pool may call; holders use `Send` to the pool.
    Transfer { recipient: String, amount: Uint128 },
    /// CW20 Send (scaled units). `contract` must be the pool so `Receive` runs (withdraw/transfer).
    Send {
        contract: String,
        amount: Uint128,
        msg: Binary,
    },
    /// Update minter and/or pool_address. Owner only.
    UpdateConfig {
        minter: Option<String>,
        pool_address: Option<String>,
    },
}

/// Empty; migrate updates cw2 contract version and may move legacy admin into cw-ownable.
#[cw_serde]
pub struct MigrateMsg {}

#[cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// CW20 Balance. Returns underlying (scaled × liquidity_index) when pool_address is set.
    #[returns(BalanceResponse)]
    Balance { address: String },
    /// Scaled balance (raw stored amount). Used by pool for admin Withdraw on behalf of lender.
    #[returns(BalanceResponse)]
    ScaledBalance { address: String },
    /// CW20 TokenInfo. total_supply is underlying when pool_address is set.
    #[returns(TokenInfoResponse)]
    TokenInfo {},
    /// CW20 Minter.
    #[returns(MinterResponse)]
    Minter {},
    /// All holders with balance greater than 0
    #[returns(AllAccountsResponse)]
    AllAccounts {
        start_after: Option<String>,
        /// Max addresses per page (CW20 `u32` limit field).
        limit: Option<u32>,
    },
}
