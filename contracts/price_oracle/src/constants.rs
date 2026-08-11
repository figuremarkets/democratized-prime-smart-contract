pub const CONTRACT_NAME: &str = "democratized_prime_price_oracle";
pub const CONTRACT_VERSION: &str = env!("CONTRACT_BUILD_VERSION");

// Re-export common constants:
pub use democratized_prime_lib::common::constants::ATTRIBUTE_ACTION_NAME;

/// Attribute key for previous prices of updated assets; value is JSON (same shape as GetPrices query).
pub const ATTRIBUTE_PREVIOUS_PRICES_JSON: &str = "previous_prices_json";

/// Attribute key for updated prices of changed assets; value is JSON (same shape as GetPrices query).
pub const ATTRIBUTE_UPDATED_PRICES_JSON: &str = "updated_prices_json";

pub const TEN: &str = "10";

/// The default price staleness threshold in seconds.
pub const DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS: u32 = 30;
