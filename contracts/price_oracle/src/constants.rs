pub const CONTRACT_NAME: &str = "democratized_prime_price_oracle";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export common constants:
pub use democratized_prime_lib::common::constants::ATTRIBUTE_ACTION_NAME;

/// Attribute key for the full asset price map; value is JSON (same shape as GetPrices query).
pub const ATTRIBUTE_PRICE_MAP_JSON: &str = "price_map_json";

pub const TEN: &str = "10";

/// The default price staleness threshold in seconds.
pub const DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS: u32 = 30;
