use serde::Serialize;

use crate::persist::GuestRegionUffdMapping;

/// Serializeable struct that contains information about guest's memory mappings
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MemoryMapingsResponse {
    /// Vector with mappings from guest physical to host virtual memoryv
    pub mappings: Vec<GuestRegionUffdMapping>,
}

