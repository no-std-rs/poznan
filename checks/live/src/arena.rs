//! Host-only bounded first-fit allocation probe. A miss falls back to System
//! solely so the probe can report it; any miss makes the probe fail.
use linked_list_allocator::Heap;
use poznan_https::RSA_HEAP_BYTES;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, UnsafeCell},
    ptr::NonNull,
    sync::Mutex,
};
thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; }
#[repr(align(16))]
struct Arena([u8; RSA_HEAP_BYTES]);
struct Storage(UnsafeCell<Arena>);
// Access to allocation metadata is serialized by METER. measure() is not concurrent.
unsafe impl Sync for Storage {}
static ARENA: Storage = Storage(UnsafeCell::new(Arena([0; RSA_HEAP_BYTES])));
fn base() -> *mut u8 {
    unsafe { std::ptr::addr_of_mut!((*ARENA.0.get()).0).cast() }
}
struct Meter {
    heap: Heap,
    peak: usize,
    misses: usize,
}
static METER: Mutex<Meter> = Mutex::new(Meter {
    heap: Heap::empty(),
    peak: 0,
    misses: 0,
});
pub struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let mut m = METER.lock().unwrap();
            if let Ok(p) = m.heap.allocate_first_fit(layout) {
                m.peak = m.peak.max(m.heap.used());
                return p.as_ptr();
            }
            m.misses += 1;
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if (base() as usize..base() as usize + RSA_HEAP_BYTES).contains(&(p as usize)) {
            unsafe {
                METER
                    .lock()
                    .unwrap()
                    .heap
                    .deallocate(NonNull::new(p).unwrap(), layout)
            };
        } else {
            unsafe { System.dealloc(p, layout) };
        }
    }
}
/// Do not let arena-backed values escape. The probe asserts all memory is freed.
pub fn measure<T>(operation: impl FnOnce() -> T) -> (T, usize, usize) {
    {
        let mut m = METER.lock().unwrap();
        assert_eq!(m.heap.used(), 0);
        m.heap = Heap::empty();
        unsafe { m.heap.init(base(), RSA_HEAP_BYTES) };
        m.peak = 0;
        m.misses = 0;
    }
    TRACK.with(|on| on.set(true));
    let result = operation();
    TRACK.with(|on| on.set(false));
    let m = METER.lock().unwrap();
    assert_eq!(m.heap.used(), 0, "probe retained arena allocations");
    (result, m.peak, m.misses)
}
