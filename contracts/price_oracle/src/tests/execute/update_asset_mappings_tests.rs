#[cfg(test)]
mod unit {
    use crate::contract::execute;
    use crate::execute::update_asset_mappings::ASSERT_OWNER_ERR;
    use crate::model::error::ContractError;
    use crate::model::SaveAssetMappingRequestV1;
    use crate::msg::ExecuteMsg;
    use crate::storage::{get_contract_state_v1, get_sorted_prices_v1, try_get_asset_mapping_v1};
    use crate::tests::constants::{
        ADMIN_ADDRESS, ALT_ASSET_ID_BTC, ALT_ASSET_ID_ETH, ALT_ASSET_ID_YLDS, ASSET_ID_BTC,
        ASSET_ID_ETH, ASSET_ID_YLDS, DENOM0, EPOCH_SECOND_JAN_01_2025, NON_ADMIN_ADDRESS,
    };
    use crate::tests::helpers::{
        mock_dependencies, mock_env_with_timestamp, AssetMappingV1Builder, ContractStateV1Builder,
    };
    use cosmwasm_std::testing::{message_info, mock_env};
    use cosmwasm_std::{coin, Addr, Response, Timestamp};
    use democratized_prime_lib::common::{illegal_argument, invalid_funds, MAX_DECIMAL_PRECISION};
    use result_extensions::ResultExtensions;

    #[test]
    fn not_admin_then_return_unauthorized_err() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(NON_ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![],
            to_remove: vec![],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert!(matches!(
            result,
            Err(ContractError::NotAuthorizedError { message })
                if message == ASSERT_OWNER_ERR
        ));

        let prices = get_sorted_prices_v1(&deps.storage, None, 100).unwrap();
        assert_eq!(prices.len(), 0);

        // No change to contract state
        assert_eq!(
            contract_state,
            get_contract_state_v1(&deps.storage).unwrap(),
        );
    }

    #[test]
    fn update_asset_mappings_rejected_with_funds() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![coin(1, "nhash")]);

        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![],
            to_remove: vec![],
        };

        let result = execute(deps.as_mut(), env, info.clone(), msg);
        assert_eq!(
            result,
            invalid_funds("No funds accepted for price oracle actions").to_err()
        );
    }

    #[test]
    fn reject_asset_mapping_if_precision_is_too_large() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let (neth_alt_id, updated_neth_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string())
            .set_asset_id(ASSET_ID_ETH.to_string())
            .set_precision(MAX_DECIMAL_PRECISION + 1) // precision too large
            .build();
        let (nbtc_alt_id, updated_nbtc_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_BTC.to_string())
            .set_asset_id(ASSET_ID_BTC.to_string())
            .set_precision(9)
            .build();

        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![
                // Not set yet
                SaveAssetMappingRequestV1 {
                    alt_asset_id: neth_alt_id,
                    mapping: updated_neth_mapping.clone(),
                },
                // Already populated
                SaveAssetMappingRequestV1 {
                    alt_asset_id: nbtc_alt_id,
                    mapping: updated_nbtc_mapping.clone(),
                },
            ],
            to_remove: vec![
                ALT_ASSET_ID_YLDS.to_string(), // Set before execution
                DENOM0.to_string(),            // Not set
            ],
        };

        let result = execute(deps.as_mut(), mock_env(), info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument(format!(
                "{ASSET_ID_ETH}: precision must be <= {MAX_DECIMAL_PRECISION}"
            ))
            .to_err()
        );
    }

    #[test]
    fn reject_duplicate_alternative_asset_mappings() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let (neth_alt_id, updated_neth_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string())
            .set_asset_id(ASSET_ID_ETH.to_string())
            .set_precision(MAX_DECIMAL_PRECISION + 1) // precision too large
            .build();
        let (nbtc_alt_id, updated_nbtc_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string()) // duplicate alt asset ID mapping
            .set_asset_id(ASSET_ID_BTC.to_string())
            .set_precision(9)
            .build();

        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![
                // Not set yet
                SaveAssetMappingRequestV1 {
                    alt_asset_id: neth_alt_id.clone(),
                    mapping: updated_neth_mapping.clone(),
                },
                // Already populated
                SaveAssetMappingRequestV1 {
                    alt_asset_id: nbtc_alt_id,
                    mapping: updated_nbtc_mapping.clone(),
                },
            ],
            to_remove: vec![
                ALT_ASSET_ID_YLDS.to_string(), // Set before execution
                DENOM0.to_string(),            // Not set
            ],
        };

        let result = execute(deps.as_mut(), mock_env(), info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument(format!("Duplicate name: {neth_alt_id}")).to_err()
        );
    }

    #[test]
    fn reject_asset_mapping_if_asset_id_is_empty() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let (neth_alt_id, updated_neth_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string())
            .set_asset_id("".to_string()) // empty
            .set_precision(9)
            .build();
        let (nbtc_alt_id, updated_nbtc_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_BTC.to_string())
            .set_asset_id(ASSET_ID_BTC.to_string())
            .set_precision(9)
            .build();

        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![
                // Not set yet
                SaveAssetMappingRequestV1 {
                    alt_asset_id: neth_alt_id,
                    mapping: updated_neth_mapping.clone(),
                },
                // Already populated
                SaveAssetMappingRequestV1 {
                    alt_asset_id: nbtc_alt_id,
                    mapping: updated_nbtc_mapping.clone(),
                },
            ],
            to_remove: vec![
                ALT_ASSET_ID_YLDS.to_string(), // Set before execution
                DENOM0.to_string(),            // Not set
            ],
        };

        let result = execute(deps.as_mut(), mock_env(), info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument("Asset ID cannot be empty").to_err()
        );
    }

    #[test]
    fn reject_asset_mapping_if_alt_asset_id_is_empty() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);

        let (neth_alt_id, updated_neth_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id("".to_string())
            .set_asset_id(ASSET_ID_ETH.to_string()) // empty
            .set_precision(9)
            .build();
        let (nbtc_alt_id, updated_nbtc_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_BTC.to_string())
            .set_asset_id(ASSET_ID_BTC.to_string())
            .set_precision(9)
            .build();

        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![
                // Not set yet
                SaveAssetMappingRequestV1 {
                    alt_asset_id: neth_alt_id,
                    mapping: updated_neth_mapping.clone(),
                },
                // Already populated
                SaveAssetMappingRequestV1 {
                    alt_asset_id: nbtc_alt_id,
                    mapping: updated_nbtc_mapping.clone(),
                },
            ],
            to_remove: vec![
                ALT_ASSET_ID_YLDS.to_string(), // Set before execution
                DENOM0.to_string(),            // Not set
            ],
        };

        let result = execute(deps.as_mut(), mock_env(), info.clone(), msg);
        assert_eq!(
            result,
            illegal_argument(format!(
                "{ASSET_ID_ETH}: alternative asset ID cannot be empty"
            ))
            .to_err()
        );
    }

    #[test]
    fn update_asset_mappings_storage() {
        let mut deps = mock_dependencies(&vec![]);
        let sender_addr = Addr::unchecked(ADMIN_ADDRESS);
        let info = message_info(&sender_addr, &vec![]);

        let _contract_state = ContractStateV1Builder::new().build_and_store(&mut deps);
        let _stored_neth_mapping = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string())
            .set_asset_id(ASSET_ID_ETH.to_string())
            .set_precision(1) // Wrong, will be updated
            .build_and_store(deps.as_mut().storage);
        let _stored_uylds_mapping = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_YLDS.to_string())
            .set_asset_id(ASSET_ID_YLDS.to_string())
            .set_precision(6)
            .build_and_store(deps.as_mut().storage);

        let (neth_alt_id, updated_neth_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_ETH.to_string())
            .set_asset_id(ASSET_ID_ETH.to_string())
            .set_precision(9)
            .build();
        let (nbtc_alt_id, updated_nbtc_mapping) = AssetMappingV1Builder::new()
            .set_alt_asset_id(ALT_ASSET_ID_BTC.to_string())
            .set_asset_id(ASSET_ID_BTC.to_string())
            .set_precision(9)
            .build();
        let msg = ExecuteMsg::UpdateAssetMappings {
            to_update: vec![
                // Not set yet
                SaveAssetMappingRequestV1 {
                    alt_asset_id: neth_alt_id,
                    mapping: updated_neth_mapping.clone(),
                },
                // Already populated
                SaveAssetMappingRequestV1 {
                    alt_asset_id: nbtc_alt_id,
                    mapping: updated_nbtc_mapping.clone(),
                },
            ],
            to_remove: vec![
                ALT_ASSET_ID_YLDS.to_string(), // Set before execution
                DENOM0.to_string(),            // Not set
            ],
        };

        let result = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
        assert_eq!(
            result,
            Response::new().add_attribute("action", "update_asset_mappings"),
        );

        let updated_neth = try_get_asset_mapping_v1(&deps.storage, &ALT_ASSET_ID_ETH).unwrap();
        assert_eq!(updated_neth, Some(updated_neth_mapping),);

        let updated_nbtc = try_get_asset_mapping_v1(&deps.storage, &ALT_ASSET_ID_BTC).unwrap();
        assert_eq!(updated_nbtc, Some(updated_nbtc_mapping),);

        let updated_uylds = try_get_asset_mapping_v1(&deps.storage, &ALT_ASSET_ID_YLDS).unwrap();
        assert!(updated_uylds.is_none());

        let updated_denom0 = try_get_asset_mapping_v1(&deps.storage, &DENOM0).unwrap();
        assert!(updated_denom0.is_none());
    }
}
