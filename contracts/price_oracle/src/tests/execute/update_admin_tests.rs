#[cfg(test)]
mod unit {
    use crate::constants::ATTRIBUTE_ACTION_NAME;
    use crate::contract::execute;
    use crate::model::error::{illegal_argument, invalid_funds, ContractError};
    use crate::msg::execute::ExecuteMsg;
    use crate::storage::get_contract_state_v1;
    use crate::tests::constants::{ADMIN_ADDRESS, EPOCH_SECOND_JAN_01_2025, NON_ADMIN_ADDRESS};
    use crate::tests::helpers::mock_env_with_timestamp;
    use crate::tests::helpers::{mock_dependencies, ContractStateV1Builder};
    use cosmwasm_std::testing::message_info;
    use cosmwasm_std::{coin, Addr, Timestamp};
    use cw_ownable::{get_ownership, Action};
    use result_extensions::ResultExtensions;

    /// Valid `tp1` bech32 distinct from [`ADMIN_ADDRESS`].
    const NEW_OWNER: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

    #[test]
    fn update_ownership_transfer_succeeds_after_accept() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        let _ = ContractStateV1Builder::new().build_and_store(&mut deps);

        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(ADMIN_ADDRESS), &[]),
            ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
                new_owner: NEW_OWNER.to_string(),
                expiry: None,
            }),
        )
        .expect("propose transfer");

        let res = execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(NEW_OWNER), &[]),
            ExecuteMsg::UpdateOwnership(Action::AcceptOwnership),
        )
        .expect("accept ownership");

        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == ATTRIBUTE_ACTION_NAME)
                .map(|a| a.value.as_str()),
            Some("update_ownership")
        );
        let o = get_ownership(deps.as_ref().storage).unwrap();
        assert_eq!(o.owner, Some(Addr::unchecked(NEW_OWNER)));
        let _ = get_contract_state_v1(&deps.storage).unwrap();
    }

    #[test]
    fn update_ownership_transfer_rejected_for_non_owner() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        ContractStateV1Builder::new().build_and_store(&mut deps);

        let result = execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(NON_ADMIN_ADDRESS), &[]),
            ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
                new_owner: NEW_OWNER.to_string(),
                expiry: None,
            }),
        );

        assert!(matches!(
            result,
            Err(ContractError::Ownership(
                cw_ownable::OwnershipError::NotOwner
            ))
        ));
    }

    #[test]
    fn update_ownership_rejected_with_funds() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        ContractStateV1Builder::new().build_and_store(&mut deps);

        let result = execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(ADMIN_ADDRESS), &[coin(1, "nhash")]),
            ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
                new_owner: NEW_OWNER.to_string(),
                expiry: None,
            }),
        );

        assert_eq!(
            result,
            invalid_funds("No funds accepted for price oracle actions").to_err()
        );
    }

    #[test]
    fn update_ownership_transfer_rejected_with_invalid_new_owner() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        ContractStateV1Builder::new().build_and_store(&mut deps);

        let err = execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(ADMIN_ADDRESS), &[]),
            ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
                new_owner: "pb1q3xhmqrjukjuhmccy4p6xza6q0uxwclled4wrf".to_string(),
                expiry: None,
            }),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ContractError::Ownership(cw_ownable::OwnershipError::Std(_))
        ));
    }

    #[test]
    fn update_ownership_renounce_rejected() {
        let mut deps = mock_dependencies(&vec![]);
        let env = mock_env_with_timestamp(Timestamp::from_seconds(EPOCH_SECOND_JAN_01_2025));
        ContractStateV1Builder::new().build_and_store(&mut deps);

        let err = execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(ADMIN_ADDRESS), &[]),
            ExecuteMsg::UpdateOwnership(Action::RenounceOwnership),
        )
        .unwrap_err();

        assert_eq!(
            err,
            illegal_argument("Renouncing contract ownership is not supported")
        );
    }
}
