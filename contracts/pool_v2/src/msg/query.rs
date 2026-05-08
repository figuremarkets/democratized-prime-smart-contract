use crate::model::{
    query::LenderStatusResponseV1, BorrowerPositionResponseV1, CollateralRequirementsResponseV1,
    ReserveResponseV1, StateResponseV1,
};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;
use cw_ownable::cw_ownable_query;

#[cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(StateResponseV1)]
    GetState {},
    /// Current reserve state (indexes, totals, utilization) plus live APRs (see `ReserveResponseV1`).
    #[returns(ReserveResponseV1)]
    GetReserve {},
    /// Borrower position: debt, collateral amounts, collateral value (USD), LTV, and health.
    #[returns(BorrowerPositionResponseV1)]
    GetBorrowerPosition { address: String },
    /// Collateral required for a given loan amount (for UI).
    #[returns(CollateralRequirementsResponseV1)]
    GetCollateralRequirements {
        borrower: Option<String>,
        new_loan_amount: Uint128,
        collateral_assets: Vec<String>,
    },
    /// Lender status: whether this address must pass commit_funds when withdrawing (per-address require commit on exit).
    /// Underlying lent position comes from the repo_token_cw20 **Balance** query, not this contract.
    #[returns(LenderStatusResponseV1)]
    GetLenderStatus { address: String },
}
