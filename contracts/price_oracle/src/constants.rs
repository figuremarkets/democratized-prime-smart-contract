pub const CONTRACT_NAME: &str = "democratized_prime_price_oracle";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export common constants:
pub use democratized_prime_lib::common::constants::ATTRIBUTE_ACTION_NAME;

pub const TEN: &str = "10";

/// The default price staleness threshold in seconds.
pub const DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS: u32 = 30;
