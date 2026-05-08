pub const CONTRACT_NAME: &str = "democratized_prime_pool_v2";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export common constants:
pub use democratized_prime_lib::common::constants::ATTRIBUTE_ACTION_NAME;

pub const MAX_LENDER_BORROWER_REQUIRED_ATTRS: usize = 10;

/// `SubMsg` reply id for instantiating the repo token CW20 from pool `instantiate`.
pub const REPO_TOKEN_INSTANTIATE_REPLY_ID: u64 = 1;
pub const ATTRIBUTE_AMOUNT: &str = "amount";
pub const ATTRIBUTE_BORROWER: &str = "borrower";
pub const ATTRIBUTE_BORROWER_REQUIRED_ATTRS_JSON: &str = "borrower_required_attrs_json";
/// Attribute key for collateral; value is JSON object (denom -> amount string). Uses `_json` suffix for JSON payloads.
pub const ATTRIBUTE_COLLATERAL_JSON: &str = "collateral_json";
pub const ATTRIBUTE_LENDER: &str = "lender";
pub const ATTRIBUTE_LENDER_REQUIRED_ATTRS_JSON: &str = "lender_required_attrs_json";
pub const ATTRIBUTE_LIQUIDATOR: &str = "liquidator";
pub const ATTRIBUTE_BORROW_RATE: &str = "borrow_rate";
pub const ATTRIBUTE_BORROW_INDEX: &str = "borrow_index";
pub const ATTRIBUTE_LEND_RATE: &str = "lend_rate";
pub const ATTRIBUTE_LIQUIDITY_INDEX: &str = "liquidity_index";
pub const ATTRIBUTE_UTILIZATION: &str = "utilization";
pub const ATTRIBUTE_RECIPIENT: &str = "recipient";
pub const ATTRIBUTE_SENDER: &str = "sender";
pub const ATTRIBUTE_SCALED_AMOUNT: &str = "scaled_amount";
/// Value is JSON array of asset ids. Uses `_json` suffix for JSON payloads.
pub const ATTRIBUTE_STATE: &str = "state";
pub const ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON: &str = "supported_collateral_updated_json";
/// Value is JSON array of asset ids. Uses `_json` suffix for JSON payloads.
pub const ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON: &str = "supported_collateral_removed_json";
pub const ATTRIBUTE_REPO_TOKEN_ADDRESS: &str = "repo_token_cw20_address";
/// Underlying amount of bad debt booked on liquidation (lending base units).
pub const ATTRIBUTE_BAD_DEBT_UNDERLYING: &str = "bad_debt_underlying";
/// Remaining reserve `deficit_underlying`.
pub const ATTRIBUTE_DEFICIT_UNDERLYING: &str = "deficit_underlying";
/// Pool config `bad_debt_loss_allocation` when a liquidation hits the bad-debt path.
pub const ATTRIBUTE_BAD_DEBT_LOSS_ALLOCATION: &str = "bad_debt_loss_allocation";
