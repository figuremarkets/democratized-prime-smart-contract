use crate::model::error::QueryError;
use cosmwasm_std::{to_json_binary, Binary};
use result_extensions::ResultExtensions;
use serde::Serialize;

pub fn query_convert_to_binary<T: Serialize>(data: &T) -> Result<Binary, QueryError> {
    to_json_binary(data).map_err(QueryError::Std)?.to_ok()
}
