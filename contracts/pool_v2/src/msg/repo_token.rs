//! Message shapes for querying and calling the repo token CW20 contract.
//! Must match repo_token_cw20's QueryMsg / ExecuteMsg for the variants we use.
//! Pool does not depend on repo_token_cw20 crate; these types serialize to the same JSON.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;

/// Query message for repo token ScaledBalance (raw stored balance; no zero balances expected).
#[cw_serde]
pub struct RepoTokenQueryMsg {
    pub scaled_balance: RepoTokenScaledBalanceQuery,
}

#[cw_serde]
pub struct RepoTokenScaledBalanceQuery {
    pub address: String,
}

/// Execute message for repo token BurnFrom (minter burns from another address).
#[cw_serde]
pub enum RepoTokenExecuteMsg {
    BurnFrom { owner: String, amount: Uint128 },
}
