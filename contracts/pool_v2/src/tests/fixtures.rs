//! Shared [`democratized_prime_lib::price_oracle::model::AssetPriceResponseV1`] fixtures for tests.
//! Expiration/`as_of` are derived from [`cosmwasm_std::Timestamp`] (typically `env.block.time` from [`cosmwasm_std::testing::mock_env`]).

use cosmwasm_std::{Decimal256, Timestamp};
use democratized_prime_lib::price_oracle::model::AssetPriceResponseV1;

pub fn fresh_oracle_price(price_usd: Decimal256, block_time: Timestamp) -> AssetPriceResponseV1 {
    let s = block_time.seconds();
    AssetPriceResponseV1 {
        as_of_epoch_second: s,
        price_usd,
        expiration_epoch_seconds: s.saturating_add(1),
    }
}

pub fn stale_oracle_price(price_usd: Decimal256, block_time: Timestamp) -> AssetPriceResponseV1 {
    let s = block_time.seconds();
    AssetPriceResponseV1 {
        as_of_epoch_second: s,
        price_usd,
        expiration_epoch_seconds: s.saturating_sub(1),
    }
}
