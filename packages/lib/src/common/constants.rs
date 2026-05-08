use cosmwasm_std::Uint64;

/// Temporal constants:
pub const SECONDS_PER_YEAR: u128 = 31_536_000_u128;
pub const SECONDS_PER_DAY: Uint64 = Uint64::new(86400_u64);
pub const SECOND_PER_HOUR: u64 = 60 * 60;

/// Common contract response attributes:
pub const ATTRIBUTE_ACTION_NAME: &str = "action";

/// The maximum representable precision for [`cosmwasm_std::Decimal256`],
/// DIGITS(10 ** 59) = 60
pub const MAX_DECIMAL_PRECISION: u32 = 59;
