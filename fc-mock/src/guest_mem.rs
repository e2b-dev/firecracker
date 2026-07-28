//! Guest memory allocation and page tracking.
//!
//! The orchestrator reads guest memory from the fc-mock process using
//! ProcessVMReadv at base_host_virt_addr. This module allocates real
//! anonymous memory and tracks which pages are resident, empty, or dirty
//! via uint64 bitmaps matching the Go models:
//!
//!   GET /memory         → { "resident": [u64], "empty": [u64] }
//!   GET /memory/dirty   → { "bitmap": [u64] }
//!   GET /memory/mappings→ { "mappings": [{ base_host_virt_addr, offset, page_size, size }] }

use antithesis_sdk::random::get_random;
use serde::Serialize;

const PAGE_SIZE: usize = 4096;

/// How much of a guest starts out resident, in bytes. Enough that the
/// orchestrator's memory export and dirty-tracking paths run against real data,
/// small enough that many concurrent sandboxes fit in the test VM's budget.
/// Override with FC_MOCK_WARM_MIB.
const DEFAULT_WARM_MIB: usize = 8;

fn warm_pages(total_pages: usize) -> usize {
    let mib = std::env::var("FC_MOCK_WARM_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_WARM_MIB);

    (mib * 1024 * 1024 / PAGE_SIZE).min(total_pages)
}

/// Bitmap with the first `set_pages` bits set.
fn prefix_bitmap(words: usize, set_pages: usize) -> Vec<u64> {
    let mut bits = vec![0u64; words];
    for page in 0..set_pages {
        bits[page / 64] |= 1u64 << (page % 64);
    }

    bits
}

pub struct GuestMemory {
    ptr: *mut u8,
    size: usize,
    total_pages: usize,
    bitmap_words: usize,
    dirty: Vec<u64>,
    resident: Vec<u64>,
    empty: Vec<u64>,
}

unsafe impl Send for GuestMemory {}
unsafe impl Sync for GuestMemory {}

impl GuestMemory {
    /// Allocate anonymous memory and initialise bitmaps.
    /// All pages start resident + dirty (fresh VM) and non-empty.
    pub fn allocate(size_mib: usize) -> Result<Self, String> {
        let size = size_mib * 1024 * 1024;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap: {}", std::io::Error::last_os_error()));
        }

        let total_pages = size / PAGE_SIZE;
        let bitmap_words = (total_pages + 63) / 64;

        // Only a prefix starts resident and dirty. Claiming the whole guest is
        // resident makes the orchestrator read and export every page, which
        // faults in the entire allocation — the mapping itself is lazy, so it is
        // the claim, not the mmap, that costs memory. A real freshly-booted
        // guest has touched a small fraction of its RAM anyway.
        let warm = warm_pages(total_pages);

        Ok(Self {
            ptr: ptr as *mut u8,
            size,
            total_pages,
            bitmap_words,
            dirty: prefix_bitmap(bitmap_words, warm),
            resident: prefix_bitmap(bitmap_words, warm),
            empty: vec![0u64; bitmap_words],
        })
    }

    /// Create from an existing allocation (e.g. from UFFD handshake).
    /// Pages start as resident but NOT dirty (restored from snapshot).
    pub fn from_existing(ptr: *mut u8, size: usize) -> Self {
        let total_pages = size / PAGE_SIZE;
        let bitmap_words = (total_pages + 63) / 64;
        let warm = warm_pages(total_pages);
        Self {
            ptr,
            size,
            total_pages,
            bitmap_words,
            dirty: vec![0u64; bitmap_words],
            resident: prefix_bitmap(bitmap_words, warm),
            empty: vec![0u64; bitmap_words],
        }
    }

    #[allow(dead_code)]
    pub fn base_addr(&self) -> u64 { self.ptr as u64 }
    #[allow(dead_code)]
    pub fn size(&self) -> usize { self.size }

    /// Mark a page as dirty (by page index).
    pub fn mark_dirty(&mut self, page_idx: usize) {
        if page_idx < self.total_pages {
            self.dirty[page_idx / 64] |= 1u64 << (page_idx % 64);
            self.resident[page_idx / 64] |= 1u64 << (page_idx % 64);
            self.empty[page_idx / 64] &= !(1u64 << (page_idx % 64));
        }
    }

    /// Mark a range of pages as dirty.
    #[allow(dead_code)]
    pub fn mark_dirty_range(&mut self, start_page: usize, count: usize) {
        for i in start_page..std::cmp::min(start_page + count, self.total_pages) {
            self.dirty[i / 64] |= 1u64 << (i % 64);
            self.resident[i / 64] |= 1u64 << (i % 64);
            self.empty[i / 64] &= !(1u64 << (i % 64));
        }
    }

    /// Touch a page in the actual allocated memory (so ProcessVMReadv
    /// sees real data) and mark it dirty.
    pub fn touch_page(&mut self, page_idx: usize, byte: u8) {
        if page_idx < self.total_pages {
            unsafe {
                let page_ptr = self.ptr.add(page_idx * PAGE_SIZE);
                *page_ptr = byte;
            }
            self.mark_dirty(page_idx);
        }
    }

    /// Touch random pages to simulate workload memory pressure.
    pub fn simulate_activity(&mut self, num_pages: usize) {
        for _ in 0..num_pages {
            let idx = (get_random() % self.total_pages as u64) as usize;
            self.touch_page(idx, get_random() as u8);
        }
    }

    /// Clear dirty bitmap (called after snapshot create).
    pub fn clear_dirty(&mut self) {
        self.dirty = vec![0u64; self.bitmap_words];
    }

    /// GET /memory/mappings response
    pub fn mappings_response(&self) -> MappingsResponse {
        MappingsResponse {
            mappings: vec![Mapping {
                base_host_virt_addr: self.ptr as i64,
                offset: 0,
                page_size: PAGE_SIZE as i64,
                size: self.size as i64,
            }],
        }
    }

    /// GET /memory response
    pub fn memory_response(&self) -> MemoryResponse {
        MemoryResponse {
            resident: self.resident.clone(),
            empty: self.empty.clone(),
        }
    }

    /// GET /memory/dirty response
    pub fn dirty_response(&self) -> DirtyResponse {
        DirtyResponse {
            bitmap: self.dirty.clone(),
        }
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size); }
    }
}

// Response shapes matching the Go generated models exactly.

#[derive(Serialize)]
pub struct MappingsResponse {
    pub mappings: Vec<Mapping>,
}

#[derive(Serialize)]
pub struct Mapping {
    pub base_host_virt_addr: i64,
    pub offset: i64,
    pub page_size: i64,
    pub size: i64,
}

#[derive(Serialize)]
pub struct MemoryResponse {
    pub resident: Vec<u64>,
    pub empty: Vec<u64>,
}

#[derive(Serialize)]
pub struct DirtyResponse {
    pub bitmap: Vec<u64>,
}
