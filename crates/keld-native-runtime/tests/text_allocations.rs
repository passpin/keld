use keld_native_runtime::RuntimeContext;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[test]
fn text_concat_allocates_only_the_result_buffer_when_a_handle_slot_is_reusable() {
    let mut context = RuntimeContext::new().expect("context");
    let lhs = context
        .text_new(b"abcdefghijklmnopqrstuvwxyz")
        .expect("lhs text");
    let rhs = context.text_new(b"!").expect("rhs text");

    // Reserve and recycle a managed handle before measurement so the concat
    // itself does not need to grow the handle table. The only required heap
    // allocation should then be the result Text byte buffer.
    let spare = context.text_new(b"spare").expect("spare text");
    context.drop_managed(spare).expect("recycle spare handle");

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let concatenated = context.text_concat(lhs, rhs).expect("concat");
    COUNTING.store(false, Ordering::Relaxed);
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);

    assert_eq!(
        allocations, 1,
        "text concat should allocate only its result byte buffer"
    );
    assert_eq!(
        context.text_bytes(concatenated),
        Ok(&b"abcdefghijklmnopqrstuvwxyz!"[..])
    );

    context.drop_managed(lhs).expect("drop lhs");
    context.drop_managed(rhs).expect("drop rhs");
    context
        .drop_managed(concatenated)
        .expect("drop concatenated text");
}
