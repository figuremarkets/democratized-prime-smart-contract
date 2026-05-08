use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct TokenInfo {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    /// Total supply in scaled units (same as sum of balances).
    pub total_supply: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    /// Address that may mint/burn (the pool once wired). Set at instantiation (including pool-driven instantiate) or via UpdateConfig.
    pub minter: Addr,
    /// Pool address to query GetReserve for liquidity_index. If None, Balance/TokenInfo return scaled.
    pub pool_address: Option<Addr>,
}

/// Scaled balances (internal accounting).
/// Zero-balance entries are removed (no zero balances expected in storage).
/// Balance query returns underlying when pool_address is set.
pub const BALANCES: Map<Addr, Uint128> = Map::new("balances");
pub const TOKEN_INFO: Item<TokenInfo> = Item::new("token_info");
pub const CONFIG: Item<Config> = Item::new("config");
