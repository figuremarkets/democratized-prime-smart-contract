use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{MemoryStorage, OwnedDeps};

pub type TestDeps = OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>;
