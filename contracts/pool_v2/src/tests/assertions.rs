use cosmwasm_std::Coin;
use provwasm_std::types::provenance::exchange::v1::MsgCommitFundsRequest;

pub fn assert_commit_matches(
    commit: &MsgCommitFundsRequest,
    account: &str,
    coins: &[Coin],
    event_tag: &str,
) {
    assert_eq!(commit.account, account);
    assert_eq!(commit.market_id, 1);
    assert_eq!(commit.amount.len(), coins.len());
    for (got, want) in commit.amount.iter().zip(coins.iter()) {
        assert_eq!(got.denom, want.denom);
        assert_eq!(got.amount, want.amount.to_string());
    }
    assert_eq!(commit.event_tag, event_tag);
}
