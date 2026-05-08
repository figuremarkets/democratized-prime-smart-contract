use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Contract instance state (reserved for future fields). Owner is stored by [`cw_ownable`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ContractStateV1 {}
