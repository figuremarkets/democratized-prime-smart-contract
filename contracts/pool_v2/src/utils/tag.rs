use crate::constants::EVENT_TAG_PREFIX;
use cosmwasm_std::Addr;

/// Generate a new event tag
///
/// All tags are of the form:
///
/// "PREFIX:$action:$account:$subaction"
pub fn new_event_tag(action: &str, account: &Addr, sub_action: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        EVENT_TAG_PREFIX,
        action,
        account.to_string(),
        sub_action
    )
}
