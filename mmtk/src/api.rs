use crate::OpenJDK;
use crate::OpenJDKEdge;
use crate::OpenJDKEdgeRange;
use crate::OpenJDK_Upcalls;
use crate::BUILDER;
use crate::SINGLETON;
use crate::UPCALLS;
use libc::c_char;
use mmtk::memory_manager;
use mmtk::plan::BarrierSelector;
use mmtk::scheduler::GCController;
use mmtk::scheduler::GCWorker;
use mmtk::util::alloc::AllocatorSelector;
use mmtk::util::constants::{LOG_BYTES_IN_ADDRESS, LOG_BYTES_IN_INT};
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::AllocationSemantics;
use mmtk::Mutator;
use mmtk::MutatorContext;
use once_cell::sync;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::sync::atomic::Ordering;

// Supported barriers:
static NO_BARRIER: sync::Lazy<CString> = sync::Lazy::new(|| CString::new("NoBarrier").unwrap());
static OBJECT_BARRIER: sync::Lazy<CString> =
    sync::Lazy::new(|| CString::new("ObjectBarrier").unwrap());
static FIELD_LOGGING_BARRIER: sync::Lazy<CString> =
    sync::Lazy::new(|| CString::new("FieldBarrier").unwrap());

#[no_mangle]
pub extern "C" fn get_mmtk_version() -> *const c_char {
    crate::build_info::MMTK_OPENJDK_FULL_VERSION.as_ptr() as _
}

#[no_mangle]
pub extern "C" fn mmtk_active_barrier() -> *const c_char {
    println!("{:?}", SINGLETON.get_plan().constraints().barrier);
    match SINGLETON.get_plan().constraints().barrier {
        BarrierSelector::NoBarrier => NO_BARRIER.as_ptr(),
        BarrierSelector::ObjectBarrier => OBJECT_BARRIER.as_ptr(),
        BarrierSelector::FieldBarrier => FIELD_LOGGING_BARRIER.as_ptr(),
        // In case we have more barriers in mmtk-core.
        #[allow(unreachable_patterns)]
        _ => unimplemented!(),
    }
}

#[no_mangle]
pub extern "C" fn mmtk_report_gc_start() {
    mmtk::memory_manager::report_gc_start::<OpenJDK>(&SINGLETON)
}

/// # Safety
/// Caller needs to make sure the ptr is a valid vector pointer.
#[no_mangle]
pub unsafe extern "C" fn release_buffer(ptr: *mut Address, length: usize, capacity: usize) {
    let _vec = Vec::<Address>::from_raw_parts(ptr, length, capacity);
}

#[no_mangle]
pub extern "C" fn openjdk_gc_init(calls: *const OpenJDK_Upcalls) {
    unsafe { UPCALLS = calls };
    crate::abi::validate_memory_layouts();

    // We don't really need this, as we can dynamically set plans. However, for compatability of our CI scripts,
    // we allow selecting a plan using feature at build time.
    // We should be able to remove this very soon.
    {
        use mmtk::util::options::PlanSelector;
        let force_plan = if cfg!(feature = "nogc") {
            Some(PlanSelector::NoGC)
        } else if cfg!(feature = "semispace") {
            Some(PlanSelector::SemiSpace)
        } else if cfg!(feature = "gencopy") {
            Some(PlanSelector::GenCopy)
        } else if cfg!(feature = "marksweep") {
            Some(PlanSelector::MarkSweep)
        } else if cfg!(feature = "markcompact") {
            Some(PlanSelector::MarkCompact)
        } else if cfg!(feature = "pageprotect") {
            Some(PlanSelector::PageProtect)
        } else if cfg!(feature = "immix") {
            Some(PlanSelector::Immix)
        } else {
            None
        };
        if let Some(plan) = force_plan {
            BUILDER.lock().unwrap().options.plan.set(plan);
        }
    }

    // Make sure that we haven't initialized MMTk (by accident) yet
    assert!(!crate::MMTK_INITIALIZED.load(Ordering::SeqCst));
    // Make sure we initialize MMTk here
    lazy_static::initialize(&SINGLETON);
}

#[no_mangle]
pub extern "C" fn openjdk_is_gc_initialized() -> bool {
    crate::MMTK_INITIALIZED.load(std::sync::atomic::Ordering::SeqCst)
}

#[no_mangle]
pub extern "C" fn mmtk_set_heap_size(size: usize) -> bool {
    let mut builder = BUILDER.lock().unwrap();
    builder.options.heap_size.set(size)
}

#[no_mangle]
pub extern "C" fn bind_mutator(tls: VMMutatorThread) -> *mut Mutator<OpenJDK> {
    Box::into_raw(memory_manager::bind_mutator(&SINGLETON, tls))
}

#[no_mangle]
// It is fine we turn the pointer back to box, as we turned a boxed value to the raw pointer in bind_mutator()
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn destroy_mutator(mutator: *mut Mutator<OpenJDK>) {
    memory_manager::destroy_mutator(unsafe { Box::from_raw(mutator) })
}

#[no_mangle]
// We trust the mutator pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn flush_mutator(mutator: *mut Mutator<OpenJDK>) {
    memory_manager::flush_mutator(unsafe { &mut *mutator })
}

#[no_mangle]
// We trust the mutator pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn alloc(
    mutator: *mut Mutator<OpenJDK>,
    size: usize,
    align: usize,
    offset: isize,
    allocator: AllocationSemantics,
) -> Address {
    memory_manager::alloc::<OpenJDK>(unsafe { &mut *mutator }, size, align, offset, allocator)
}

#[no_mangle]
pub extern "C" fn get_allocator_mapping(allocator: AllocationSemantics) -> AllocatorSelector {
    memory_manager::get_allocator_mapping(&SINGLETON, allocator)
}

#[no_mangle]
pub extern "C" fn get_max_non_los_default_alloc_bytes() -> usize {
    SINGLETON
        .get_plan()
        .constraints()
        .max_non_los_default_alloc_bytes
}

#[no_mangle]
// We trust the mutator pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn post_alloc(
    mutator: *mut Mutator<OpenJDK>,
    refer: ObjectReference,
    bytes: usize,
    allocator: AllocationSemantics,
) {
    memory_manager::post_alloc::<OpenJDK>(unsafe { &mut *mutator }, refer, bytes, allocator)
}

#[no_mangle]
pub extern "C" fn will_never_move(object: ObjectReference) -> bool {
    !object.is_movable()
}

#[no_mangle]
// We trust the gc_collector pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn start_control_collector(
    tls: VMWorkerThread,
    gc_controller: *mut GCController<OpenJDK>,
) {
    let mut gc_controller = unsafe { Box::from_raw(gc_controller) };
    memory_manager::start_control_collector(&SINGLETON, tls, &mut gc_controller);
}

#[no_mangle]
// We trust the worker pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn start_worker(tls: VMWorkerThread, worker: *mut GCWorker<OpenJDK>) {
    crate::CURRENT_WORKER.with(|x| x.replace(Some(unsafe { &mut *worker })));
    let mut worker = unsafe { Box::from_raw(worker) };
    memory_manager::start_worker::<OpenJDK>(&SINGLETON, tls, &mut worker)
}

#[no_mangle]
pub extern "C" fn initialize_collection(tls: VMThread) {
    memory_manager::initialize_collection(&SINGLETON, tls)
}

#[no_mangle]
pub extern "C" fn used_bytes() -> usize {
    memory_manager::used_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn free_bytes() -> usize {
    memory_manager::free_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn total_bytes() -> usize {
    memory_manager::total_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn handle_user_collection_request(tls: VMMutatorThread) {
    memory_manager::handle_user_collection_request::<OpenJDK>(&SINGLETON, tls);
}

#[no_mangle]
pub extern "C" fn mmtk_use_compressed_ptrs() {
    crate::init_compressed_oop_constants()
}

#[no_mangle]
pub extern "C" fn is_in_mmtk_spaces(object: ObjectReference) -> bool {
    memory_manager::is_in_mmtk_spaces(object)
}

#[no_mangle]
pub extern "C" fn is_mapped_address(addr: Address) -> bool {
    memory_manager::is_mapped_address(addr)
}

#[no_mangle]
pub extern "C" fn modify_check(object: ObjectReference) {
    memory_manager::modify_check(&SINGLETON, object)
}

#[no_mangle]
pub extern "C" fn add_weak_candidate(reff: ObjectReference) {
    memory_manager::add_weak_candidate(&SINGLETON, reff)
}

#[no_mangle]
pub extern "C" fn add_soft_candidate(reff: ObjectReference) {
    memory_manager::add_soft_candidate(&SINGLETON, reff)
}

#[no_mangle]
pub extern "C" fn add_phantom_candidate(reff: ObjectReference) {
    memory_manager::add_phantom_candidate(&SINGLETON, reff)
}

// The harness_begin()/end() functions are different than other API functions in terms of the thread state.
// Other functions are called by the VM, thus the thread should already be in the VM state. But the harness
// functions are called by the probe, and the thread is in JNI/application/native state. Thus we need call
// into VM to switch the thread state and VM will then call into mmtk-core again to do the actual work of
// harness_begin() and harness_end()

#[no_mangle]
pub extern "C" fn harness_begin(_id: usize) {
    unsafe { ((*UPCALLS).harness_begin)() };
}

#[no_mangle]
pub extern "C" fn mmtk_harness_begin_impl() {
    // Pass null as tls, OpenJDK binding does not rely on the tls value to block the current thread and do a GC
    memory_manager::harness_begin(&SINGLETON, VMMutatorThread(VMThread::UNINITIALIZED));
}

#[no_mangle]
pub extern "C" fn harness_end(_id: usize) {
    unsafe { ((*UPCALLS).harness_end)() };
}

#[no_mangle]
pub extern "C" fn mmtk_harness_end_impl() {
    memory_manager::harness_end(&SINGLETON);
}

#[no_mangle]
// We trust the name/value pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn process(name: *const c_char, value: *const c_char) -> bool {
    let name_str: &CStr = unsafe { CStr::from_ptr(name) };
    let value_str: &CStr = unsafe { CStr::from_ptr(value) };
    let mut builder = BUILDER.lock().unwrap();
    memory_manager::process(
        &mut builder,
        name_str.to_str().unwrap(),
        value_str.to_str().unwrap(),
    )
}

#[no_mangle]
// We trust the name/value pointer is valid.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn process_bulk(options: *const c_char) -> bool {
    let options_str: &CStr = unsafe { CStr::from_ptr(options) };
    let mut builder = BUILDER.lock().unwrap();
    memory_manager::process_bulk(&mut builder, options_str.to_str().unwrap())
}

#[no_mangle]
pub extern "C" fn mmtk_narrow_oop_base() -> Address {
    unsafe { crate::BASE }
}

#[no_mangle]
pub extern "C" fn mmtk_narrow_oop_shift() -> usize {
    unsafe { crate::SHIFT }
}

#[no_mangle]
pub extern "C" fn starting_heap_address() -> Address {
    memory_manager::starting_heap_address()
}

#[no_mangle]
pub extern "C" fn last_heap_address() -> Address {
    memory_manager::last_heap_address()
}

#[no_mangle]
pub extern "C" fn openjdk_max_capacity() -> usize {
    memory_manager::total_bytes(&SINGLETON)
}

#[no_mangle]
pub extern "C" fn executable() -> bool {
    true
}

#[no_mangle]
pub extern "C" fn mmtk_load_reference(mutator: &'static mut Mutator<OpenJDK>, o: ObjectReference) {
    mutator.barrier().load_reference(o);
}

#[no_mangle]
pub extern "C" fn mmtk_object_reference_clone_pre(
    mutator: &'static mut Mutator<OpenJDK>,
    obj: ObjectReference,
) {
    mutator.barrier().object_reference_clone_pre(obj);
}

/// Full pre barrier
#[no_mangle]
pub extern "C" fn mmtk_object_reference_write_pre(
    mutator: &'static mut Mutator<OpenJDK>,
    src: ObjectReference,
    slot: Address,
    target: ObjectReference,
) {
    mutator
        .barrier()
        .object_reference_write_pre(src, OpenJDKEdge(slot), target);
}

/// Full post barrier
#[no_mangle]
pub extern "C" fn mmtk_object_reference_write_post(
    mutator: &'static mut Mutator<OpenJDK>,
    src: ObjectReference,
    slot: Address,
    target: ObjectReference,
) {
    mutator
        .barrier()
        .object_reference_write_post(src, OpenJDKEdge(slot), target);
}

/// Barrier slow-path call
#[no_mangle]
pub extern "C" fn mmtk_object_reference_write_slow(
    mutator: &'static mut Mutator<OpenJDK>,
    src: ObjectReference,
    slot: Address,
    target: ObjectReference,
) {
    mutator
        .barrier()
        .object_reference_write_slow(src, OpenJDKEdge(slot), target);
}

/// Array-copy pre-barrier
#[no_mangle]
pub extern "C" fn mmtk_array_copy_pre(
    mutator: &'static mut Mutator<OpenJDK>,
    src: Address,
    dst: Address,
    count: usize,
) {
    let bytes = count
        << if crate::use_compressed_oops() {
            LOG_BYTES_IN_INT
        } else {
            LOG_BYTES_IN_ADDRESS
        };
    mutator.barrier().memory_region_copy_pre(
        OpenJDKEdgeRange {
            start: OpenJDKEdge(src),
            end: OpenJDKEdge(src + bytes),
        },
        OpenJDKEdgeRange {
            start: OpenJDKEdge(dst),
            end: OpenJDKEdge(dst + bytes),
        },
    );
}

/// Array-copy post-barrier
#[no_mangle]
pub extern "C" fn mmtk_array_copy_post(
    mutator: &'static mut Mutator<OpenJDK>,
    src: Address,
    dst: Address,
    count: usize,
) {
    let bytes = count
        << if crate::use_compressed_oops() {
            LOG_BYTES_IN_INT
        } else {
            LOG_BYTES_IN_ADDRESS
        };
    mutator.barrier().memory_region_copy_post(
        OpenJDKEdgeRange {
            start: OpenJDKEdge(src),
            end: OpenJDKEdge(src + bytes),
        },
        OpenJDKEdgeRange {
            start: OpenJDKEdge(dst),
            end: OpenJDKEdge(dst + bytes),
        },
    );
}

// finalization
#[no_mangle]
pub extern "C" fn add_finalizer(_object: ObjectReference) {
    unreachable!()
}

#[no_mangle]
pub extern "C" fn get_finalized_object() -> ObjectReference {
    match memory_manager::get_finalized_object(&SINGLETON) {
        Some(obj) => obj,
        None => unsafe { Address::ZERO.to_object_reference() },
    }
}

/// Test if an object is live at the end of a GC.
/// Note: only call this method after the liveness tracing and before gc release.
#[no_mangle]
pub extern "C" fn mmtk_is_live(object: ObjectReference) -> usize {
    if object.is_null() {
        return 0;
    }
    debug_assert!(object.to_address().is_mapped());
    debug_assert!(object.class_is_valid::<OpenJDK>());
    object.is_live() as _
}

/// If the object is non-null and forwarded, return the forwarded pointer. Otherwise, return the original pointer.
#[no_mangle]
pub extern "C" fn mmtk_get_forwarded_ref(object: ObjectReference) -> ObjectReference {
    if object.is_null() {
        return object;
    }
    object.get_forwarded_object().unwrap_or(object)
}

thread_local! {
    /// Cache all the pointers reported by the current thread.
    static NMETHOD_SLOTS: RefCell<Vec<Address>> = RefCell::new(vec![]);
}

/// Report a list of pointers in nmethod to mmtk.
#[no_mangle]
pub extern "C" fn mmtk_add_nmethod_oop(addr: Address) {
    NMETHOD_SLOTS.with(|x| x.borrow_mut().push(addr))
}

/// Register a nmethod.
/// The c++ part of the binding should scan the nmethod and report all the pointers to mmtk first, before calling this function.
/// This function will transfer all the locally cached pointers of this nmethod to the global storage.
#[no_mangle]
pub extern "C" fn mmtk_register_nmethod(nm: Address) {
    let slots = NMETHOD_SLOTS.with(|x| {
        if x.borrow().len() == 0 {
            return None;
        }
        Some(x.replace(vec![]))
    });
    let slots = match slots {
        Some(slots) => slots,
        _ => return,
    };
    let mut roots = crate::CODE_CACHE_ROOTS.lock().unwrap();
    // Relaxed add instead of `fetch_add`, since we've already acquired the lock.
    crate::CODE_CACHE_ROOTS_SIZE.store(
        crate::CODE_CACHE_ROOTS_SIZE.load(Ordering::Relaxed) + slots.len(),
        Ordering::Relaxed,
    );
    roots.insert(nm, slots);
}

/// Unregister a nmethod.
#[no_mangle]
pub extern "C" fn mmtk_unregister_nmethod(nm: Address) {
    let mut roots = crate::CODE_CACHE_ROOTS.lock().unwrap();
    if let Some(slots) = roots.remove(&nm) {
        // Relaxed sub instead of `fetch_sub`, since we've already acquired the lock.
        crate::CODE_CACHE_ROOTS_SIZE.store(
            crate::CODE_CACHE_ROOTS_SIZE.load(Ordering::Relaxed) - slots.len(),
            Ordering::Relaxed,
        );
    }
}
