use cosmwasm_schema::cw_serde;

#[cw_serde]
pub struct MigrateMsg {
    /// If provided, the account to set as the contract custodian.
    ///
    /// _Note:_ when migrating an existing contract that does not have a
    /// custodian accouint set, this value __MUST__ be provided, otherwise
    /// and error will be raised and the migration will fail.
    pub custodian: Option<String>,
}
