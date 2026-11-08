//! Counts allocations, reallocations and bytes (debug builds only), for
//! Console → Resources and the allocation audit tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call goes straight to the system allocator; only counters are added.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        FREES.fetch_add(1, Relaxed);
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        REALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(size.saturating_sub(layout.size()) as u64, Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Totals since start.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub allocs: u64,
    pub reallocs: u64,
    pub frees: u64,
    pub bytes: u64,
}

pub fn counts() -> Counts {
    Counts { allocs: ALLOCS.load(Relaxed), reallocs: REALLOCS.load(Relaxed), frees: FREES.load(Relaxed), bytes: BYTES.load(Relaxed) }
}

impl std::ops::Sub for Counts {
    type Output = Counts;
    fn sub(self, before: Counts) -> Counts {
        Counts { allocs: self.allocs - before.allocs, reallocs: self.reallocs - before.reallocs, frees: self.frees - before.frees, bytes: self.bytes - before.bytes }
    }
}
