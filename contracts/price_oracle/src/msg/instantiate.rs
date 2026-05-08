use cosmwasm_schema::cw_serde;

#[cw_serde]
pub struct InstantiateMsg {
    /// Initial contract owner (same role as legacy `admin` JSON field).
    #[serde(alias = "admin")]
    pub owner: String,
}
