use serde::Serialize;

use crate::persist::GuestRegionUffdMapping;

/// Serializeable struct that contains information about guest's memory mappings
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MemoryMapingsResponse {
    /// Vector with mappings from guest physical to host virtual memoryv
    pub mappings: Vec<GuestRegionUffdMapping>,
}

/// Information about guest memory resident pages and pages that are all-0s
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MemoryResponse {
    /// Bitmap for resident pages. The bitmap is encoded as a vector of u64 values.
    /// Each bit represents whether a page is present in the resident memory set
    pub resident: Vec<u64>,
    /// Bitmap for empty pages. The bitmap is encoded as a vector of u64 values.
    /// Each bit represents whether a page is empty (all 0s).
    pub empty: Vec<u64>,
}
