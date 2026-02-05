use micro_http::{Method, StatusCode};
use vmm::rpc_interface::VmmAction;

use crate::api_server::parsed_request::{ParsedRequest, RequestError};

pub(crate) fn parse_get_memory<'a, T>(mut path_tokens: T) -> Result<ParsedRequest, RequestError>
where
    T: Iterator<Item = &'a str>,
{
    match path_tokens.next() {
        Some("mappings") => Ok(ParsedRequest::new_sync(VmmAction::GetMemoryMappings)),
        Some(unknown_path) => Err(RequestError::InvalidPathMethod(
            format!("/memory/{}", unknown_path),
            Method::Get,
        )),
        None => Err(RequestError::Generic(
            StatusCode::BadRequest,
            "Missing memory info type.".to_string(),
        )),
    }
}
