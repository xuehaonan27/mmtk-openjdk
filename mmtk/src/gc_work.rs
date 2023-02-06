use crate::scanning::to_edges_closure;
use crate::Address;
use crate::EdgesClosure;
use crate::NewBuffer;
use crate::{OpenJDK, OpenJDKEdge, UPCALLS};
use mmtk::scheduler::*;
use mmtk::vm::RootsWorkFactory;
use mmtk::MMTK;

macro_rules! scan_roots_work {
    ($struct_name: ident, $func_name: ident) => {
        pub struct $struct_name<F: RootsWorkFactory<OpenJDKEdge>> {
            factory: F,
        }

        impl<F: RootsWorkFactory<OpenJDKEdge>> $struct_name<F> {
            pub fn new(factory: F) -> Self {
                Self { factory }
            }
        }

        impl<F: RootsWorkFactory<OpenJDKEdge>> GCWork<OpenJDK> for $struct_name<F> {
            fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
                unsafe {
                    ((*UPCALLS).$func_name)(to_edges_closure(&mut self.factory));
                }
            }
        }
    };
}

scan_roots_work!(ScanUniverseRoots, scan_universe_roots);
scan_roots_work!(ScanJNIHandlesRoots, scan_jni_handle_roots);
scan_roots_work!(ScanObjectSynchronizerRoots, scan_object_synchronizer_roots);
scan_roots_work!(ScanManagementRoots, scan_management_roots);
scan_roots_work!(ScanJvmtiExportRoots, scan_jvmti_export_roots);
scan_roots_work!(ScanAOTLoaderRoots, scan_aot_loader_roots);
scan_roots_work!(ScanSystemDictionaryRoots, scan_system_dictionary_roots);
scan_roots_work!(ScanStringTableRoots, scan_string_table_roots);
scan_roots_work!(ScanVMThreadRoots, scan_vm_thread_roots);

pub struct ScanClassLoaderDataGraphRoots<F: RootsWorkFactory<OpenJDKEdge>> {
    factory: F,
}

impl<F: RootsWorkFactory<OpenJDKEdge>> ScanClassLoaderDataGraphRoots<F> {
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

extern "C" fn report_edges_and_renew_buffer_cld<
    F: RootsWorkFactory<OpenJDKEdge>,
    const WEAK: bool,
>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut OpenJDKEdge;
        let buf = unsafe { Vec::<OpenJDKEdge>::from_raw_parts(ptr, length, capacity) };
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_edge_roots_work_for_cld_roots(buf, WEAK);
    }
    let (ptr, _, capacity) = {
        // TODO: Use Vec::into_raw_parts() when the method is available.
        use std::mem::ManuallyDrop;
        let new_vec = Vec::with_capacity(F::BUFFER_SIZE);
        let mut me = ManuallyDrop::new(new_vec);
        (me.as_mut_ptr(), me.len(), me.capacity())
    };
    NewBuffer { ptr, capacity }
}

fn to_edges_closure_cld<F: RootsWorkFactory<OpenJDKEdge>, const WEAK: bool>(
    factory: &mut F,
) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_cld::<F, WEAK>,
        data: factory as *mut F as *mut libc::c_void,
    }
}
impl<F: RootsWorkFactory<OpenJDKEdge>> GCWork<OpenJDK> for ScanClassLoaderDataGraphRoots<F> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_class_loader_data_graph_roots)(
                to_edges_closure_cld::<F, false>(&mut self.factory),
                to_edges_closure_cld::<F, true>(&mut self.factory),
                mmtk.get_plan()
                    .current_gc_should_scan_weak_classloader_roots(),
            );
        }
    }
}

pub struct ScanCodeCacheRoots<F: RootsWorkFactory<OpenJDKEdge>> {
    factory: F,
}

impl<F: RootsWorkFactory<OpenJDKEdge>> ScanCodeCacheRoots<F> {
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F: RootsWorkFactory<OpenJDKEdge>> GCWork<OpenJDK> for ScanCodeCacheRoots<F> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        // Collect all the cached roots
        let mut edges = Vec::with_capacity(F::BUFFER_SIZE);
        for roots in (*crate::CODE_CACHE_ROOTS.lock().unwrap()).values() {
            for r in roots {
                edges.push(OpenJDKEdge(*r));
                if edges.len() >= F::BUFFER_SIZE {
                    self.factory
                        .create_process_edge_roots_work(std::mem::take(&mut edges));
                    edges.reserve(F::BUFFER_SIZE);
                }
            }
        }
        // Create work packet
        if !edges.is_empty() {
            self.factory.create_process_edge_roots_work(edges);
        }
        // Use the following code to scan CodeCache directly, instead of scanning the "remembered set".
        // unsafe {
        //     ((*UPCALLS).scan_code_cache_roots)(create_process_edges_work::<E> as _);
        // }
    }
}
