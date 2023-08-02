use mmtk::scheduler::{GCWorker, ProcessEdgesWork};
use mmtk::util::alloc::AllocationError;
use mmtk::util::opaque_pointer::*;
use mmtk::vm::{Collection, GCThreadContext, Scanning, VMBinding};
use mmtk::{Mutator, MutatorContext};

use crate::reference_glue::DISCOVERED_LISTS;
use crate::UPCALLS;
use crate::{MutatorClosure, OpenJDK};

pub struct VMCollection {}

const GC_THREAD_KIND_CONTROLLER: libc::c_int = 0;
const GC_THREAD_KIND_WORKER: libc::c_int = 1;

impl<const COMPRESSED: bool> Collection<OpenJDK<COMPRESSED>> for VMCollection {
    fn stop_all_mutators<F>(
        tls: VMWorkerThread,
        mut mutator_visitor: F,
        current_gc_should_unload_classes: bool,
    ) where
        F: FnMut(&'static mut Mutator<OpenJDK<COMPRESSED>>),
    {
        let scan_mutators_in_safepoint =
            <<OpenJDK<COMPRESSED> as VMBinding>::VMScanning as Scanning<OpenJDK<COMPRESSED>>>::SCAN_MUTATORS_IN_SAFEPOINT;

        unsafe {
            ((*UPCALLS).stop_all_mutators)(
                tls,
                scan_mutators_in_safepoint,
                MutatorClosure::from_rust_closure::<_, COMPRESSED>(&mut mutator_visitor),
                current_gc_should_unload_classes,
            );
        }
    }

    fn resume_mutators(tls: VMWorkerThread) {
        if cfg!(feature = "object_size_distribution") {
            crate::dump_and_reset_obj_dist();
        }
        DISCOVERED_LISTS.enable_discover();
        unsafe {
            ((*UPCALLS).resume_mutators)(tls);
        }
    }

    fn block_for_gc(_tls: VMMutatorThread) {
        unsafe {
            ((*UPCALLS).block_for_gc)();
        }
    }

    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<OpenJDK<COMPRESSED>>) {
        let (ctx_ptr, kind) = match ctx {
            GCThreadContext::Controller(c) => (
                Box::into_raw(c) as *mut libc::c_void,
                GC_THREAD_KIND_CONTROLLER,
            ),
            GCThreadContext::Worker(w) => {
                (Box::into_raw(w) as *mut libc::c_void, GC_THREAD_KIND_WORKER)
            }
        };
        unsafe {
            ((*UPCALLS).spawn_gc_thread)(tls, kind, ctx_ptr);
        }
    }

    fn prepare_mutator<T: MutatorContext<OpenJDK<COMPRESSED>>>(
        _tls_w: VMWorkerThread,
        _tls_m: VMMutatorThread,
        _m: &T,
    ) {
        // unimplemented!()
    }

    fn out_of_memory(tls: VMThread, err_kind: AllocationError) {
        unsafe {
            ((*UPCALLS).out_of_memory)(tls, err_kind);
        }
    }

    fn schedule_finalization(_tls: VMWorkerThread) {
        unreachable!()
    }

    fn process_weak_refs<E: ProcessEdgesWork<VM = OpenJDK<COMPRESSED>>>(
        worker: &mut GCWorker<OpenJDK<COMPRESSED>>,
    ) {
        DISCOVERED_LISTS.process_soft_weak_final_refs::<E>(worker)
    }

    fn process_final_refs<E: ProcessEdgesWork<VM = OpenJDK<COMPRESSED>>>(
        worker: &mut GCWorker<OpenJDK<COMPRESSED>>,
    ) {
        DISCOVERED_LISTS.resurrect_final_refs::<E>(worker)
    }

    fn process_phantom_refs<E: ProcessEdgesWork<VM = OpenJDK<COMPRESSED>>>(
        worker: &mut GCWorker<OpenJDK<COMPRESSED>>,
    ) {
        DISCOVERED_LISTS.process_phantom_refs::<E>(worker)
    }

    fn update_weak_processor(lxr: bool) {
        unsafe {
            ((*UPCALLS).update_weak_processor)(lxr);
        }
    }

    fn clear_cld_claimed_marks() {
        unsafe {
            ((*UPCALLS).clear_claimed_marks)();
        }
    }

    fn set_concurrent_marking_state(active: bool) {
        unsafe { crate::CONCURRENT_MARKING_ACTIVE = if active { 1 } else { 0 } }
    }

    fn vm_release(do_unloading: bool) {
        unsafe {
            if do_unloading {
                ((*UPCALLS).unload_classes)();
            }
            ((*UPCALLS).gc_epilogue)();
        }
    }
}
