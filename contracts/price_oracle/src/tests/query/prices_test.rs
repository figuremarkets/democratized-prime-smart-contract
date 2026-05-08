#[cfg(test)]
mod query_prices_by_asset_unit {
    use crate::constants::DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS;
    use crate::model::error::QueryError;
    use crate::query::query_prices_by_assets;
    use crate::tests::constants::EPOCH_SECOND_JAN_01_2025;
    use crate::tests::helpers::{mock_dependencies, AssetMappingV1Builder, PriceV1Builder};
    use cosmwasm_std::{from_json, Decimal256, Storage};
    use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
    use result_extensions::ResultExtensions;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn setup_mapping(store: &mut dyn Storage, staleness_threshold_seconds: u32) {
        AssetMappingV1Builder::new()
            .set_asset_id("BTC")
            .set_alt_asset_id("nbtc.figure.se")
            .set_precision(9)
            .set_staleness_threshold_seconds(staleness_threshold_seconds)
            .build_and_store(store);
    }

    fn setup_price(store: &mut dyn Storage, as_of_time: u64) {
        PriceV1Builder::new()
            .set_asset_id("BTC")
            .set_as_of_time(as_of_time)
            .set_price_usd(&Decimal256::from_str("100000.123").unwrap())
            .build_and_store(store);
    }

    #[test]
    fn get_by_alt_asset_id_price_returns_price_map() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // price is 10 seconds old:
        setup_price(deps.as_mut().storage, EPOCH_SECOND_JAN_01_2025 - 10);

        let result = query_prices_by_assets(&deps.storage, vec![String::from("nbtc.figure.se")]);
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();

        let mut expected: PriceMapResponse = HashMap::new();
        expected.insert(
            "nbtc.figure.se".to_string(),
            AssetPriceResponseV1 {
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025 - 10,
                price_usd: Decimal256::from_str("0.000100000123").unwrap(),
                expiration_epoch_seconds: (EPOCH_SECOND_JAN_01_2025 - 10)
                    + (DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS as u64),
            },
        );
        assert_eq!(result_body, expected)
    }

    #[test]
    fn get_by_display_asset_id_price_returns_price_map() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // price is 10 seconds old:
        setup_price(deps.as_mut().storage, EPOCH_SECOND_JAN_01_2025 - 10);

        let result = query_prices_by_assets(&deps.storage, vec![String::from("BTC")]);
        let result_body: PriceMapResponse = from_json(result.unwrap()).unwrap();

        let mut expected: PriceMapResponse = HashMap::new();
        expected.insert(
            "BTC".to_string(),
            AssetPriceResponseV1 {
                as_of_epoch_second: EPOCH_SECOND_JAN_01_2025 - 10,
                price_usd: Decimal256::from_str("100000.123").unwrap(),
                expiration_epoch_seconds: (EPOCH_SECOND_JAN_01_2025 - 10)
                    + (DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS as u64),
            },
        );
        assert_eq!(result_body, expected,)
    }

    #[test]
    fn mapping_found_but_price_not_found_return_error() {
        let mut deps = mock_dependencies(&[]);

        setup_mapping(
            deps.as_mut().storage,
            DEFAULT_PRICE_STALENESS_THRESHOLD_SECONDS,
        );
        // No price for BTC

        let result = query_prices_by_assets(&deps.storage, vec![String::from("nbtc.figure.se")]);
        assert_eq!(
            result,
            QueryError::NotFoundError {
                message: "BTC".to_owned()
            }
            .to_err()
        );
    }

    #[test]
    fn mapping_not_found_and_price_not_found_return_error() {
        let deps = mock_dependencies(&[]);

        // No mapping for nbtc.figure.se
        // No price for BTC

        let result = query_prices_by_assets(
            &deps.storage,
            vec![String::from("nbtc.figure.se"), String::from("BTC")],
        );
        assert_eq!(
            result,
            QueryError::NotFoundError {
                message: "nbtc.figure.se".to_owned()
            }
            .to_err()
        );
    }
}
