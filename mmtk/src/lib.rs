extern crate libc;
extern crate mmtk;
#[macro_use]
extern crate lazy_static;
extern crate atomic;
extern crate once_cell;
extern crate spin;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::null_mut;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use libc::{c_char, c_void, uintptr_t};
use mmtk::scheduler::GCWorker;
use mmtk::util::alloc::AllocationError;
use mmtk::util::constants::{
    BYTES_IN_ADDRESS, BYTES_IN_INT, LOG_BYTES_IN_ADDRESS, LOG_BYTES_IN_INT,
};
use mmtk::util::heap::layout::vm_layout_constants::VM_LAYOUT_CONSTANTS;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::edge_shape::{Edge, MemorySlice};
use mmtk::vm::VMBinding;
use mmtk::MMTKBuilder;
use mmtk::Mutator;
use mmtk::MMTK;

mod abi;
pub mod active_plan;
pub mod api;
mod build_info;
pub mod collection;
mod gc_work;
pub mod object_model;
mod object_scanning;
pub mod reference_glue;
pub mod scanning;
pub(crate) mod vm_metadata;

#[repr(C)]
pub struct NewBuffer {
    pub ptr: *mut Address,
    pub capacity: usize,
}

/// A closure for reporting mutators.  The C++ code should pass `data` back as the last argument.
#[repr(C)]
pub struct MutatorClosure {
    pub func: *const extern "C" fn(mutator: *mut Mutator<OpenJDK>, data: *mut libc::c_void),
    pub data: *mut libc::c_void,
}

/// A closure for reporting root edges.  The C++ code should pass `data` back as the last argument.
#[repr(C)]
pub struct EdgesClosure {
    pub func:
        *const extern "C" fn(buf: *mut Address, size: usize, cap: usize, data: *const libc::c_void),
    pub data: *const libc::c_void,
}

#[repr(C)]
pub struct OpenJDK_Upcalls {
    pub stop_all_mutators: extern "C" fn(
        tls: VMWorkerThread,
        scan_mutators_in_safepoint: bool,
        closure: MutatorClosure,
        current_gc_should_unload_classes: bool,
    ),
    pub resume_mutators:
        extern "C" fn(tls: VMWorkerThread, lxr: bool, current_gc_should_unload_classes: bool),
    pub spawn_gc_thread: extern "C" fn(tls: VMThread, kind: libc::c_int, ctx: *mut libc::c_void),
    pub block_for_gc: extern "C" fn(),
    pub out_of_memory: extern "C" fn(tls: VMThread, err_kind: AllocationError),
    pub get_next_mutator: extern "C" fn() -> *mut Mutator<OpenJDK>,
    pub reset_mutator_iterator: extern "C" fn(),
    pub scan_object: extern "C" fn(
        trace: *mut c_void,
        object: ObjectReference,
        tls: OpaquePointer,
        follow_clds: bool,
        claim_clds: bool,
    ),
    pub dump_object: extern "C" fn(object: ObjectReference),
    pub get_object_size: extern "C" fn(object: ObjectReference) -> usize,
    pub get_mmtk_mutator: extern "C" fn(tls: VMMutatorThread) -> *mut Mutator<OpenJDK>,
    pub is_mutator: extern "C" fn(tls: VMThread) -> bool,
    pub harness_begin: extern "C" fn(),
    pub harness_end: extern "C" fn(),
    pub compute_klass_mem_layout_checksum: extern "C" fn() -> usize,
    pub offset_of_static_fields: extern "C" fn() -> i32,
    pub static_oop_field_count_offset: extern "C" fn() -> i32,
    pub referent_offset: extern "C" fn() -> i32,
    pub discovered_offset: extern "C" fn() -> i32,
    pub dump_object_string: extern "C" fn(object: ObjectReference) -> *const c_char,
    pub scan_all_thread_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_thread_roots: extern "C" fn(closure: EdgesClosure, tls: VMMutatorThread),
    pub scan_universe_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_jni_handle_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_object_synchronizer_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_management_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_jvmti_export_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_aot_loader_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_system_dictionary_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_code_cache_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_string_table_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_class_loader_data_graph_roots: extern "C" fn(closure: EdgesClosure, scan_weak: bool),
    pub scan_weak_processor_roots: extern "C" fn(closure: EdgesClosure),
    pub scan_vm_thread_roots: extern "C" fn(closure: EdgesClosure),
    pub number_of_mutators: extern "C" fn() -> usize,
    pub schedule_finalizer: extern "C" fn(),
    pub prepare_for_roots_re_scanning: extern "C" fn(),
    pub update_weak_processor: extern "C" fn(lxr: bool),
    pub enqueue_references: extern "C" fn(objects: *const ObjectReference, len: usize),
    pub swap_reference_pending_list: extern "C" fn(objects: ObjectReference) -> ObjectReference,
    pub java_lang_class_klass_offset_in_bytes: extern "C" fn() -> usize,
    pub java_lang_classloader_loader_data_offset: extern "C" fn() -> usize,
    pub compressed_klass_base: extern "C" fn() -> Address,
    pub compressed_klass_shift: extern "C" fn() -> usize,
    pub nmethod_fix_relocation: extern "C" fn(Address),
}

lazy_static! {
    pub static ref JAVA_LANG_CLASS_KLASS_OFFSET_IN_BYTES: usize =
        unsafe { ((*UPCALLS).java_lang_class_klass_offset_in_bytes)() };
    pub static ref JAVA_LANG_CLASSLOADER_LOADER_DATA_OFFSET: usize =
        unsafe { ((*UPCALLS).java_lang_classloader_loader_data_offset)() };
}

thread_local! {
    pub static CURRENT_WORKER: RefCell<Option<*mut GCWorker<OpenJDK>>> = RefCell::new(None);
}

#[inline(always)]
pub fn current_worker() -> &'static mut GCWorker<OpenJDK> {
    CURRENT_WORKER.with(|x| {
        let ptr = x.borrow().unwrap();
        unsafe { &mut *ptr }
    })
}

pub static mut UPCALLS: *const OpenJDK_Upcalls = null_mut();

#[no_mangle]
pub static GLOBAL_SIDE_METADATA_BASE_ADDRESS: uintptr_t =
    crate::mmtk::util::metadata::side_metadata::GLOBAL_SIDE_METADATA_BASE_ADDRESS.as_usize();

#[no_mangle]
pub static GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS: uintptr_t =
    crate::mmtk::util::metadata::side_metadata::GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS.as_usize();

#[no_mangle]
pub static GLOBAL_ALLOC_BIT_ADDRESS: uintptr_t =
    crate::mmtk::util::metadata::side_metadata::ALLOC_SIDE_METADATA_ADDR.as_usize();

#[no_mangle]
pub static DISABLE_ALLOCATION_FAST_PATH: i32 = cfg!(feature = "no_fast_alloc") as _;

#[no_mangle]
pub static IMMIX_ALLOCATOR_SIZE: uintptr_t =
    std::mem::size_of::<mmtk::util::alloc::ImmixAllocator<OpenJDK>>();

#[no_mangle]
pub static mut CONCURRENT_MARKING_ACTIVE: u8 = 0;

static mut USE_COMPRESSED_OOPS: bool = false;

#[inline(always)]
fn use_compressed_oops() -> bool {
    unsafe { USE_COMPRESSED_OOPS }
}

static mut BASE: Address = Address::ZERO;
static mut SHIFT: usize = 0;

fn compress(o: ObjectReference) -> u32 {
    if o.is_null() {
        0u32
    } else {
        unsafe { ((o.to_address() - BASE) >> SHIFT) as u32 }
    }
}

fn decompress(v: u32) -> ObjectReference {
    if v == 0 {
        ObjectReference::NULL
    } else {
        unsafe { (BASE + ((v as usize) << SHIFT)).to_object_reference() }
    }
}

fn initialize_compressed_oops() {
    let heap_end = VM_LAYOUT_CONSTANTS.heap_end.as_usize();
    if heap_end <= (4usize << 30) {
        unsafe {
            BASE = Address::ZERO;
            SHIFT = 0;
        }
    } else if heap_end <= (32usize << 30) {
        unsafe {
            BASE = Address::ZERO;
            SHIFT = 3;
        }
    } else {
        unsafe {
            BASE = VM_LAYOUT_CONSTANTS.heap_start - 4096;
            SHIFT = 3;
        }
    }
}

#[derive(Default)]
pub struct OpenJDK;

/// The type of edges in OpenJDK.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
#[repr(transparent)]
pub struct OpenJDKEdge(pub Address);

impl OpenJDKEdge {
    const MASK: usize = 1usize << 63;

    const fn is_compressed(&self) -> bool {
        self.0.as_usize() & Self::MASK == 0
    }

    const fn untagged_address(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() << 1 >> 1) }
    }
}

impl Edge for OpenJDKEdge {
    /// Load object reference from the edge.
    #[inline(always)]
    fn load<const COMPRESSED: bool>(&self) -> ObjectReference {
        if COMPRESSED {
            let slot = self.untagged_address();
            if self.is_compressed() {
                decompress(unsafe { slot.load::<u32>() })
            } else {
                unsafe { slot.load::<ObjectReference>() }
            }
        } else {
            unsafe { self.0.load::<ObjectReference>() }
        }
    }

    /// Store the object reference `object` into the edge.
    #[inline(always)]
    fn store<const COMPRESSED: bool>(&self, object: ObjectReference) {
        if COMPRESSED {
            let slot = self.untagged_address();
            if self.is_compressed() {
                unsafe { slot.store(compress(object)) }
            } else {
                unsafe { slot.store(object) }
            }
        } else {
            unsafe { self.0.store(object) }
        }
    }

    #[inline(always)]
    fn to_address(&self) -> Address {
        self.untagged_address()
    }

    #[inline(always)]
    fn from_address(a: Address) -> Self {
        unreachable!();
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct OpenJDKEdgeRange {
    pub start: OpenJDKEdge,
    pub end: OpenJDKEdge,
}

/// Iterate edges within `Range<Address>`.
pub struct AddressRangeIterator {
    cursor: Address,
    limit: Address,
    width: usize,
}

impl Iterator for AddressRangeIterator {
    type Item = OpenJDKEdge;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.limit {
            None
        } else {
            let edge = self.cursor;
            self.cursor += self.width;
            Some(OpenJDKEdge(edge))
        }
    }
}

impl MemorySlice for OpenJDKEdgeRange {
    type Edge = OpenJDKEdge;
    type EdgeIterator = AddressRangeIterator;

    #[inline]
    fn iter_edges(&self) -> Self::EdgeIterator {
        AddressRangeIterator {
            cursor: self.start.0,
            limit: self.end.0,
            width: if crate::use_compressed_oops() {
                BYTES_IN_INT
            } else {
                BYTES_IN_ADDRESS
            },
        }
    }

    #[inline]
    fn start(&self) -> Address {
        self.start.0
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.end.0 - self.start.0
    }

    #[inline]
    fn copy(src: &Self, tgt: &Self) {
        debug_assert_eq!(src.bytes(), tgt.bytes());
        debug_assert_eq!(
            src.bytes() & ((1 << LOG_BYTES_IN_ADDRESS) - 1),
            0,
            "bytes are not a multiple of words"
        );
        // Raw memory copy
        if crate::use_compressed_oops() {
            unsafe {
                let words = tgt.bytes() >> LOG_BYTES_IN_INT;
                let src = src.start().to_ptr::<u32>();
                let tgt = tgt.start().to_mut_ptr::<u32>();
                std::ptr::copy(src, tgt, words)
            }
        } else {
            unsafe {
                let words = tgt.bytes() >> LOG_BYTES_IN_ADDRESS;
                let src = src.start().to_ptr::<usize>();
                let tgt = tgt.start().to_mut_ptr::<usize>();
                std::ptr::copy(src, tgt, words)
            }
        }
    }
}

impl VMBinding for OpenJDK {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;

    type VMEdge = OpenJDKEdge;
    type VMMemorySlice = OpenJDKEdgeRange;
}

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub static MMTK_INITIALIZED: AtomicBool = AtomicBool::new(false);

lazy_static! {
    pub static ref BUILDER: Mutex<MMTKBuilder> = Mutex::new(MMTKBuilder::new());
    pub static ref SINGLETON: MMTK<OpenJDK> = {
        let mut builder = BUILDER.lock().unwrap();
        if use_compressed_oops() {
            builder.set_option("use_35bit_address_space", "true");
            builder.set_option("use_35bit_address_space", "true");
        }
        assert!(!MMTK_INITIALIZED.load(Ordering::Relaxed));
        let ret = mmtk::memory_manager::mmtk_init(&builder);
        MMTK_INITIALIZED.store(true, std::sync::atomic::Ordering::SeqCst);
        if use_compressed_oops() {
            initialize_compressed_oops();
        }
        *ret
    };
}

#[no_mangle]
pub static MMTK_MARK_COMPACT_HEADER_RESERVED_IN_BYTES: usize =
    mmtk::util::alloc::MarkCompactAllocator::<OpenJDK>::HEADER_RESERVED_IN_BYTES;

lazy_static! {
    /// A global storage for all the cached CodeCache root pointers
    static ref CODE_CACHE_ROOTS: Mutex<HashMap<Address, Vec<Address>>> = Mutex::new(HashMap::new());
}

/// A counter tracking the total size of the `CODE_CACHE_ROOTS`.
static CODE_CACHE_ROOTS_SIZE: AtomicUsize = AtomicUsize::new(0);
