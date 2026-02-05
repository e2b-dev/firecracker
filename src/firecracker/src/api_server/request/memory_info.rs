use vmm::rpc_interface::VmmAction;

use crate::api_server::parsed_request::{ParsedRequest, RequestError};

pub(crate) fn parse_get_guest_memory_mappings() -> Result<ParsedRequest, RequestError> {
    // TODO: Define the metric and then use it here
    // METRICS.get_api_requests.guest_memory_mappings_count.inc();
    Ok(ParsedRequest::new_sync(VmmAction::GetMemoryMappings))
}
