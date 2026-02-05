use micro_http::StatusCode;
use vmm::rpc_interface::VmmAction;

use crate::api_server::parsed_request::{ParsedRequest, RequestError};

pub(crate) fn parse_get_memory<'a, T>(mut path_tokens: T) -> Result<ParsedRequest, RequestError>
where
    T: Iterator<Item = &'a str>,
{
    match path_tokens.next() {
        Some("mappings") => Ok(ParsedRequest::new_sync(VmmAction::GetMemoryMappings)),
        Some(unknown_path) => Err(RequestError::Generic(
            StatusCode::BadRequest,
            format!("Unrecognized GET request path `{unknown_path}`"),
        )),
        None => Ok(ParsedRequest::new_sync(VmmAction::GetMemory)),
    }
}
