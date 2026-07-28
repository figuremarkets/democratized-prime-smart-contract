use cosmwasm_std::Uint128;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Extensions to the CW20 `token_info` response.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ExtendedTokenInfoResponse {
    /// The name of the token.
    pub name: String,
    /// The symbol of the token.
    pub symbol: String,
    /// The decimal precision of the token.
    pub decimals: u8,
    /// The total supply of the token. If the contract has a configured pool address,
    /// the total supply will be returned in underlying units, otherwise scaled units.
    pub total_supply: Uint128,
    /// The total supply of the token in scaled units.
    pub total_scaled_supply: Uint128,
}
