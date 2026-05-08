//! Contract identity and version for cw2 versioning and migrate.

pub const CONTRACT_NAME: &str = "repo-token-cw20";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default page size for `QueryMsg::AllAccounts` when `limit` is omitted.
pub const DEFAULT_ALL_ACCOUNTS_PAGE_SIZE: u32 = 100;

/// Upper bound on `limit` for `QueryMsg::AllAccounts` (explicit values are clamped).
pub const MAX_ALL_ACCOUNTS_PAGE_SIZE: u32 = 100;
