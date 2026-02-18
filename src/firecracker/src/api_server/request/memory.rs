// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use vmm::logger::{IncMetric, METRICS};
use vmm::rpc_interface::VmmAction;

use super::super::parsed_request::{ParsedRequest, RequestError};

pub(crate) fn parse_get_memory_mappings() -> Result<ParsedRequest, RequestError> {
    METRICS.get_api_requests.instance_info_count.inc();
    Ok(ParsedRequest::new_sync(VmmAction::GetMemoryMappings))
}

pub(crate) fn parse_get_memory() -> Result<ParsedRequest, RequestError> {
    METRICS.get_api_requests.instance_info_count.inc();
    Ok(ParsedRequest::new_sync(VmmAction::GetMemory))
}

pub(crate) fn parse_get_memory_dirty() -> Result<ParsedRequest, RequestError> {
    METRICS.get_api_requests.instance_info_count.inc();
    Ok(ParsedRequest::new_sync(VmmAction::GetMemoryDirty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_server::parsed_request::RequestAction;

    #[test]
    fn test_parse_get_memory_mappings_request() {
        match parse_get_memory_mappings().unwrap().into_parts() {
            (RequestAction::Sync(action), _) if *action == VmmAction::GetMemoryMappings => {}
            _ => panic!("Test failed."),
        }
    }

    #[test]
    fn test_parse_get_memory_request() {
        match parse_get_memory().unwrap().into_parts() {
            (RequestAction::Sync(action), _) if *action == VmmAction::GetMemory => {}
            _ => panic!("Test failed."),
        }
    }

    #[test]
    fn test_parse_get_memory_dirty_request() {
        match parse_get_memory_dirty().unwrap().into_parts() {
            (RequestAction::Sync(action), _) if *action == VmmAction::GetMemoryDirty => {}
            _ => panic!("Test failed."),
        }
    }
}
