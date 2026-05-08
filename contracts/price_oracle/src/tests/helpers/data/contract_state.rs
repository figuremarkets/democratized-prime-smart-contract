use crate::storage::contract_state::set_contract_state_v1;
use crate::tests::constants::ADMIN_ADDRESS;
use cosmwasm_std::testing::{MockApi, MockStorage};
use cosmwasm_std::{Addr, Empty, OwnedDeps};
use democratized_prime_lib::price_oracle::model::ContractStateV1;
use provwasm_mocks::MockProvenanceQuerier;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractStateV1Builder {
    admin: Addr,
}

impl ContractStateV1Builder {
    pub fn new() -> Self {
        Self {
            admin: Addr::unchecked(ADMIN_ADDRESS),
        }
    }
    #[allow(dead_code)]
    pub fn set_admin(mut self, admin: Addr) -> Self {
        self.admin = admin;
        self
    }
    #[allow(dead_code)]
    pub fn build(self) -> ContractStateV1 {
        ContractStateV1 {}
    }
    #[allow(dead_code)]
    pub fn build_and_store(
        self,
        deps: &mut OwnedDeps<MockStorage, MockApi, MockProvenanceQuerier, Empty>,
    ) -> ContractStateV1 {
        let owner = self.admin.clone();
        let contract = ContractStateV1 {};
        let d = deps.as_mut();
        set_contract_state_v1(d.storage, &contract).unwrap();
        cw_ownable::initialize_owner(d.storage, d.api, Some(owner.as_str())).unwrap();
        contract
    }
}
