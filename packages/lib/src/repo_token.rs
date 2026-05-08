//! Shared repo (receipt) CW20 **instantiate** message and validation used by `repo_token_cw20` and
//! `pool_v2` (SubMsg JSON must match the contract’s `instantiate` schema).

use cosmwasm_schema::cw_serde;

/// JSON for **`repo_token_cw20`** `instantiate` and for **`pool_v2`**’s `WasmMsg::Instantiate` submessage when creating the repo token in the same transaction.
#[cw_serde]
pub struct InstantiateMsg {
    pub name: String,
    pub symbol: String,
    /// Token decimals (typically same as lending denom, e.g. 6 for uylds.fcc).
    pub decimals: u8,
    /// Initial owner (may transfer via `cw_ownable`); may call UpdateConfig to set minter/pool_address when the token was created standalone.
    #[serde(alias = "admin")]
    pub owner: String,
    /// Minter (only this address may mint/burn). Often the pool; may start as admin if the repo is deployed before the pool, then **UpdateConfig** wires the pool.
    pub minter: String,
    /// Pool address for Balance/TokenInfo to return underlying (`GetReserve` / liquidity index). Set at instantiate when pool_v2 uses **`repo_token.new`**, or later via **UpdateConfig** on the “existing token” path.
    pub pool_address: Option<String>,
}

/// Validate name/symbol/decimals to match cw20-base conventions (see CW20_AUDIT.md in repo_token_cw20).
pub fn validate_repo_token_meta(
    name: &str,
    symbol: &str,
    decimals: u8,
) -> Result<(), &'static str> {
    let name_len = name.len();
    if !(3..=50).contains(&name_len) {
        return Err("name must be 3–50 UTF-8 bytes");
    }
    let symbol_bytes = symbol.as_bytes();
    if symbol_bytes.len() < 3 || symbol_bytes.len() > 12 {
        return Err("symbol must be 3–12 bytes");
    }
    for &b in symbol_bytes {
        if b != b'-' && !b.is_ascii_uppercase() && !b.is_ascii_lowercase() {
            return Err("symbol must contain only [a-zA-Z\\-]");
        }
    }
    if decimals > 18 {
        return Err("decimals must not exceed 18");
    }
    Ok(())
}
