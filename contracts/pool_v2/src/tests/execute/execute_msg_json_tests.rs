//! Each test uses an explicit JSON string (the wire format the contract deserializes from `msg`)
//! and asserts it deserializes to the expected `ExecuteMsg`.

use crate::model::{CollateralAssetV1, OperationalState, RateParamsV1};
use crate::msg::execute::{Cw20ReceivePayload, EliminateDeficitFunding};
use crate::msg::ExecuteMsg;
use cosmwasm_std::{to_json_binary, Binary, Decimal256, Uint128};
use cw20::Cw20ReceiveMsg;
use cw_ownable::Action;
use std::collections::BTreeMap;
use std::str::FromStr;

fn assert_json_deserializes(json: &str, expected: ExecuteMsg) {
    let got: ExecuteMsg = serde_json::from_str(json).unwrap_or_else(|e| {
        panic!(
            "failed to deserialize ExecuteMsg from JSON: {}\nJSON:\n{}",
            e, json
        )
    });
    assert_eq!(got, expected, "JSON:\n{}", json);
}

#[test]
fn lend_json_deserializes() {
    assert_json_deserializes(r#"{"lend":{}}"#, ExecuteMsg::Lend {});
}

#[test]
fn receive_withdraw_json_deserializes() {
    assert_json_deserializes(
        r#"{"receive":{"sender":"tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu","amount":"1000000","msg":"eyJ3aXRoZHJhdyI6eyJhbW91bnQiOiI1MDAwMDAifX0="}}"#,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu".to_string(),
            amount: Uint128::new(1_000_000),
            msg: Binary::from(r#"{"withdraw":{"amount":"500000"}}"#.as_bytes()),
        }),
    );
}

#[test]
fn receive_withdraw_exact_json_deserializes() {
    assert_json_deserializes(
        r#"{"receive":{"sender":"tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu","amount":"1000000","msg":"eyJ3aXRoZHJhd19leGFjdCI6e319"}}"#,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu".to_string(),
            amount: Uint128::new(1_000_000),
            msg: Binary::from(r#"{"withdraw_exact":{}}"#.as_bytes()),
        }),
    );
}

#[test]
fn receive_transfer_json_deserializes() {
    assert_json_deserializes(
        r#"{"receive":{"sender":"tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu","amount":"1000000","msg":"eyJ0cmFuc2ZlciI6eyJyZWNpcGllbnQiOiJ0cDF0a24yZHdma3g3cG1qcjJydGdxaHRydWRzdjdoOHcydGo2ZWVzdiIsImFtb3VudCI6IjEwMCJ9fQ=="}}"#,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu".to_string(),
            amount: Uint128::new(1_000_000),
            msg: to_json_binary(&Cw20ReceivePayload::Transfer {
                recipient: "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv".to_string(),
                amount: Uint128::new(100),
            })
            .unwrap(),
        }),
    );
}

#[test]
fn receive_transfer_exact_json_deserializes() {
    assert_json_deserializes(
        r#"{"receive":{"sender":"tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu","amount":"1000000","msg":"eyJ0cmFuc2Zlcl9leGFjdCI6eyJyZWNpcGllbnQiOiJ0cDF0a24yZHdma3g3cG1qcjJydGdxaHRydWRzdjdoOHcydGo2ZWVzdiJ9fQ=="}}"#,
        ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "tp1q8n4v4m0hm8v0a7n697nwtpzhfsz3f4d40lnsu".to_string(),
            amount: Uint128::new(1_000_000),
            msg: to_json_binary(&Cw20ReceivePayload::TransferExact {
                recipient: "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv".to_string(),
            })
            .unwrap(),
        }),
    );
}

#[test]
fn borrow_json_deserializes() {
    assert_json_deserializes(
        r#"{"borrow":{"amount":"42"}}"#,
        ExecuteMsg::Borrow {
            amount: Uint128::new(42),
        },
    );
}

#[test]
fn repay_json_deserializes() {
    assert_json_deserializes(r#"{"repay":{}}"#, ExecuteMsg::Repay {});
}

#[test]
fn add_collateral_json_deserializes() {
    assert_json_deserializes(r#"{"add_collateral":{}}"#, ExecuteMsg::AddCollateral {});
}

#[test]
fn remove_collateral_json_deserializes() {
    let mut to_remove = BTreeMap::new();
    to_remove.insert("asset.one".to_string(), Uint128::new(100));
    assert_json_deserializes(
        r#"{"remove_collateral":{"to_remove":{"asset.one":"100"}}}"#,
        ExecuteMsg::RemoveCollateral { to_remove },
    );
}

#[test]
fn liquidate_json_deserializes() {
    let mut collateral_to_seize = BTreeMap::new();
    collateral_to_seize.insert("asset.one".to_string(), Uint128::new(50));
    assert_json_deserializes(
        r#"{"liquidate":{"borrower":"tp1borrowerxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","collateral_to_seize":{"asset.one":"50"}}}"#,
        ExecuteMsg::Liquidate {
            borrower: "tp1borrowerxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            collateral_to_seize,
        },
    );
}

#[test]
fn update_supported_collateral_json_deserializes() {
    assert_json_deserializes(
        r#"{"update_supported_collateral":{"to_update":[{"id":"asset.one","h":"0.8"}],"to_remove":["old.asset"]}}"#,
        ExecuteMsg::UpdateSupportedCollateral {
            to_update: vec![CollateralAssetV1 {
                asset_id: "asset.one".to_string(),
                haircut: Some(Decimal256::from_str("0.8").unwrap()),
            }],
            to_remove: vec!["old.asset".to_string()],
        },
    );
}

#[test]
fn withdraw_reserve_recipient_none_json_deserializes() {
    assert_json_deserializes(
        r#"{"withdraw_reserve":{"recipient":null}}"#,
        ExecuteMsg::WithdrawReserve { recipient: None },
    );
}

#[test]
fn withdraw_reserve_recipient_some_json_deserializes() {
    assert_json_deserializes(
        r#"{"withdraw_reserve":{"recipient":"tp1recipientxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}}"#,
        ExecuteMsg::WithdrawReserve {
            recipient: Some("tp1recipientxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string()),
        },
    );
}

#[test]
fn eliminate_deficit_accrued_reserve_json_deserializes() {
    assert_json_deserializes(
        r#"{"eliminate_deficit":{"funding":{"accrued_reserve":{"max_underlying":"1000000"}}}}"#,
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::AccruedReserve {
                max_underlying: Uint128::new(1_000_000),
            },
        },
    );
}

#[test]
fn eliminate_deficit_bank_json_deserializes() {
    assert_json_deserializes(
        r#"{"eliminate_deficit":{"funding":{"bank":{"max_underlying":"42"}}}}"#,
        ExecuteMsg::EliminateDeficit {
            funding: EliminateDeficitFunding::Bank {
                max_underlying: Uint128::new(42),
            },
        },
    );
}

#[test]
fn set_operational_state_active_json_deserializes() {
    assert_json_deserializes(
        r#"{"set_operational_state":{"state":"active"}}"#,
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Active,
        },
    );
}

#[test]
fn set_operational_state_frozen_json_deserializes() {
    assert_json_deserializes(
        r#"{"set_operational_state":{"state":"frozen"}}"#,
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Frozen,
        },
    );
}

#[test]
fn set_operational_state_paused_json_deserializes() {
    assert_json_deserializes(
        r#"{"set_operational_state":{"state":"paused"}}"#,
        ExecuteMsg::SetOperationalState {
            state: OperationalState::Paused,
        },
    );
}

#[test]
fn update_ownership_transfer_json_deserializes() {
    assert_json_deserializes(
        r#"{"update_ownership":{"transfer_ownership":{"new_owner":"tp1newownerxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","expiry":null}}}"#,
        ExecuteMsg::UpdateOwnership(Action::TransferOwnership {
            new_owner: "tp1newownerxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            expiry: None,
        }),
    );
}

#[test]
fn set_lender_required_attrs_json_deserializes() {
    assert_json_deserializes(
        r#"{"set_lender_required_attrs":{"lender_required_attrs":["kyc.passed","accredited"]}}"#,
        ExecuteMsg::SetLenderRequiredAttrs {
            lender_required_attrs: vec!["kyc.passed".to_string(), "accredited".to_string()],
        },
    );
}

#[test]
fn set_borrower_required_attrs_json_deserializes() {
    assert_json_deserializes(
        r#"{"set_borrower_required_attrs":{"borrower_required_attrs":["kyc.passed"]}}"#,
        ExecuteMsg::SetBorrowerRequiredAttrs {
            borrower_required_attrs: vec!["kyc.passed".to_string()],
        },
    );
}

#[test]
fn update_contract_config_json_deserializes() {
    assert_json_deserializes(
        r#"{"update_contract_config":{"margin_rate":"0.75","liquidation_rate":null,"liquidation_bonus_rate":"1.05","price_oracle_address":"tp1oraclexxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","min_lend":"10","min_borrow":null,"max_borrower_collateral_types":8}}"#,
        ExecuteMsg::UpdateContractConfig {
            margin_rate: Some(Decimal256::from_str("0.75").unwrap()),
            liquidation_rate: None,
            liquidation_bonus_rate: Some(Decimal256::from_ratio(105u128, 100u128)),
            price_oracle_address: Some("tp1oraclexxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string()),
            min_lend: Some(Uint128::new(10)),
            min_borrow: None,
            max_borrower_collateral_types: Some(8),
            commit_market_id: None,
            bad_debt_loss_allocation: None,
        },
    );
}

#[test]
fn update_contract_config_json_deserializes_bad_debt_allocation() {
    use crate::model::BadDebtLossAllocation;
    assert_json_deserializes(
        r#"{"update_contract_config":{"bad_debt_loss_allocation":"immediate_liquidity_index_haircut"}}"#,
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Some(BadDebtLossAllocation::ImmediateLiquidityIndexHaircut),
        },
    );
}

#[test]
fn update_contract_config_json_deserializes_commit_market_id() {
    assert_json_deserializes(
        r#"{"update_contract_config":{"commit_market_id":42}}"#,
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(42),
            bad_debt_loss_allocation: None,
        },
    );
}

#[test]
fn socialize_deficit_json_deserializes() {
    assert_json_deserializes(
        r#"{"socialize_deficit":{"max_amount":"1000"}}"#,
        ExecuteMsg::SocializeDeficit {
            max_amount: Uint128::new(1000),
        },
    );
}

#[test]
fn update_rate_params_json_deserializes() {
    assert_json_deserializes(
        r#"{"update_rate_params":{"rate_params":{"tr":"0.09","minr":"0.0325","maxr":"0.2","kink":"0.9","rf":"0.005","spy":31536000}}}"#,
        ExecuteMsg::UpdateRateParams {
            rate_params: RateParamsV1 {
                target_rate: Decimal256::from_str("0.09").unwrap(),
                min_rate: Decimal256::from_str("0.0325").unwrap(),
                max_rate: Decimal256::from_str("0.20").unwrap(),
                kink_utilization: Decimal256::from_str("0.90").unwrap(),
                reserve_factor: Decimal256::from_str("0.005").unwrap(),
                seconds_per_year: 31_536_000,
            },
        },
    );
}

#[test]
fn withdraw_amount_none_json_deserializes() {
    assert_json_deserializes(
        r#"{"withdraw":{"lender":"tp1lenderxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","amount":null}}"#,
        ExecuteMsg::Withdraw {
            lender: "tp1lenderxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            amount: None,
            commit_funds: None,
        },
    );
}

#[test]
fn withdraw_amount_some_json_deserializes() {
    assert_json_deserializes(
        r#"{"withdraw":{"lender":"tp1lenderxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","amount":"1000000"}}"#,
        ExecuteMsg::Withdraw {
            lender: "tp1lenderxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            amount: Some(Uint128::new(1_000_000)),
            commit_funds: None,
        },
    );
}
