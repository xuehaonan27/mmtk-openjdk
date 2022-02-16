use std::sync::atomic::Ordering;

use super::gc_work::*;
use super::{NewBuffer, SINGLETON, UPCALLS};
use crate::OpenJDK;
use mmtk::memory_manager;
use mmtk::scheduler::ProcessEdgesWork;
use mmtk::scheduler::{GCWorker, WorkBucketStage};
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::Scanning;
use mmtk::MutatorContext;
use mmtk::{Mutator, TransitiveClosure};

pub struct VMScanning {}


pub(crate) extern "C" fn create_process_edges_work_mu<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::STACK_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub(crate) extern "C" fn create_process_edges_work<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        let buf = unsafe { Vec::<Address>::from_raw_parts(ptr, length, capacity) };
        let mut c = 0usize;
        for a in &buf {
            if unsafe { a.load::<usize>() != 0 } {
                c += 1;
            }
        }
        memory_manager::add_work_packet(
            &SINGLETON,
            WorkBucketStage::Closure,
            W::new(buf, true, &SINGLETON),
        );
        mmtk::NON_NULL_ROOTS.fetch_add(c, Ordering::SeqCst);
        mmtk::ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    let (ptr, _, capacity) = Vec::with_capacity(W::CAPACITY).into_raw_parts();
    NewBuffer { ptr, capacity }
}

impl Scanning<OpenJDK> for VMScanning {
    const SCAN_MUTATORS_IN_SAFEPOINT: bool = false;
    const SINGLE_THREAD_MUTATOR_SCANNING: bool = false;

    fn scan_object<T: TransitiveClosure>(
        trace: &mut T,
        object: ObjectReference,
        tls: VMWorkerThread,
    ) {
        crate::object_scanning::scan_object(object, trace, tls)
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: VMWorkerThread) {
        // unimplemented!()
        // TODO
    }

    fn scan_objects<W: ProcessEdgesWork<VM = OpenJDK>>(
        objects: &[ObjectReference],
        worker: &mut GCWorker<OpenJDK>,
    ) {
        crate::object_scanning::scan_objects_and_create_edges_work::<W>(objects, worker);
    }

    fn scan_thread_roots<W: ProcessEdgesWork<VM = OpenJDK>>() {
        let process_edges = create_process_edges_work_mu::<W>;
        unsafe {
            ((*UPCALLS).scan_thread_roots)(process_edges as _);
        }
    }

    fn scan_thread_root<W: ProcessEdgesWork<VM = OpenJDK>>(
        mutator: &'static mut Mutator<OpenJDK>,
        _tls: VMWorkerThread,
    ) {
        let tls = mutator.get_tls();
        let process_edges = create_process_edges_work_mu::<W>;
        unsafe {
            ((*UPCALLS).scan_thread_root)(process_edges as _, tls);
        }
    }

    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM = OpenJDK>>() {
        memory_manager::add_work_packets(
            &SINGLETON,
            WorkBucketStage::Prepare,
            vec![
                box ScanUniverseRoots::<W>::new(),
                box ScanJNIHandlesRoots::<W>::new(),
                box ScanObjectSynchronizerRoots::<W>::new(),
                box ScanManagementRoots::<W>::new(),
                box ScanJvmtiExportRoots::<W>::new(),
                box ScanAOTLoaderRoots::<W>::new(),
                box ScanSystemDictionaryRoots::<W>::new(),
                box ScanCodeCacheRoots::<W>::new(),
                box ScanStringTableRoots::<W>::new(),
                box ScanClassLoaderDataGraphRoots::<W>::new(),
                box ScanWeakProcessorRoots::<W>::new(),
            ],
        );
        if !(Self::SCAN_MUTATORS_IN_SAFEPOINT && Self::SINGLE_THREAD_MUTATOR_SCANNING) {
            memory_manager::add_work_packet(
                &SINGLETON,
                WorkBucketStage::Prepare,
                ScanVMThreadRoots::<W>::new(),
            );
        }
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }

    fn prepare_for_roots_re_scanning() {
        unsafe {
            ((*UPCALLS).prepare_for_roots_re_scanning)();
        }
    }
}
