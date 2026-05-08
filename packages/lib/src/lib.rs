//! Shared types for Democratized Prime CosmWasm contracts.
//!
//! - **`repo_token`** — `InstantiateMsg` and validation for the **`repo_token_cw20`** contract; also used by **`pool_v2`** when instantiating that CW20 in a SubMsg.

pub mod common;
pub mod pool;
pub mod price_oracle;
pub mod repo_token;
#[cfg(test)]
pub mod tests;
