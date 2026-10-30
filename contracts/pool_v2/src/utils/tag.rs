use crate::constants::EVENT_TAG_PREFIX;
use cosmwasm_std::Addr;

/// Generate a new event tag
///
/// All tags are of the form:
///
/// "PREFIX:$action:$account"
pub fn new_event_tag(action: &str, account: &Addr) -> String {
    format!("{}:{}:{}", EVENT_TAG_PREFIX, action, account)
}
