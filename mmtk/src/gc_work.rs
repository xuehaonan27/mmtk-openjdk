use crate::scanning::to_edges_closure;
use crate::Address;
use crate::Edge;
use crate::EdgesClosure;
use crate::NewBuffer;
use crate::UPCALLS;
use mmtk::scheduler::*;
use mmtk::util::ObjectReference;
use mmtk::vm::RootsWorkFactory;
use mmtk::vm::*;
use mmtk::MMTK;
use std::collections::HashSet;
use std::marker::PhantomData;

macro_rules! scan_roots_work {
    ($struct_name: ident, $func_name: ident) => {
        pub struct $struct_name<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> {
            factory: F,
            _p: std::marker::PhantomData<VM>,
        }

        impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> $struct_name<VM, F> {
            pub fn new(factory: F) -> Self {
                Self {
                    factory,
                    _p: std::marker::PhantomData,
                }
            }
        }

        impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM> for $struct_name<VM, F> {
            fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
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

extern "C" fn report_edges_and_renew_buffer_cld<
    E: Edge,
    F: RootsWorkFactory<E>,
    const WEAK: bool,
>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    let root_kind = if WEAK {
        RootKind::Weak
    } else {
        RootKind::Young
    };
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_edge_roots_work(buf, root_kind);
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

fn to_edges_closure_cld<E: Edge, F: RootsWorkFactory<E>, const WEAK: bool>(
    factory: &mut F,
) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_cld::<E, F, WEAK>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

pub struct ScanClassLoaderDataGraphRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanClassLoaderDataGraphRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanClassLoaderDataGraphRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        unsafe {
            ((*UPCALLS).scan_class_loader_data_graph_roots)(
                to_edges_closure_cld::<VM::VMEdge, F, false>(&mut self.factory),
                to_edges_closure_cld::<VM::VMEdge, F, true>(&mut self.factory),
                mmtk.get_plan()
                    .current_gc_should_scan_all_classloader_strong_roots(),
            );
        }
    }
}

extern "C" fn report_edges_and_renew_buffer_code<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_edge_roots_work(buf, RootKind::Young);
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

fn to_edges_closure_code<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_code::<E, F>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

pub struct ScanCodeCacheRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanCodeCacheRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanCodeCacheRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let scan_all_roots = mmtk
            .get_plan()
            .current_gc_should_scan_all_classloader_strong_roots()
            || mmtk.get_plan().current_gc_should_perform_class_unloading();
        let mut all_nmethods = vec![];
        let mut mature = crate::MATURE_CODE_CACHE_ROOTS.lock().unwrap();
        let mut nursery_guard = crate::NURSERY_CODE_CACHE_ROOTS.lock().unwrap();
        let nursery = std::mem::take::<HashSet<Address>>(&mut nursery_guard);
        if scan_all_roots {
            // Collect all the mature cached roots
            for nm in mature.iter() {
                all_nmethods.push(*nm);
            }
        }
        // Young roots
        for nm in nursery {
            all_nmethods.push(nm);
            mature.insert(nm);
        }
        unsafe {
            let ptr = all_nmethods.as_ptr();
            let len = all_nmethods.len();
            ((*UPCALLS).scan_code_cache_roots2)(
                ptr,
                len,
                to_edges_closure_code::<_, _>(&mut self.factory),
            );
        }
    }
}

extern "C" fn report_edges_and_renew_buffer_weakref<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let buf = unsafe {
            Vec::<ObjectReference>::from_raw_parts(ptr as *mut ObjectReference, length, capacity)
        };
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_node_roots_work(buf, RootKind::Weak);
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

fn to_edges_closure_weakref<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_weakref::<E, F>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

pub struct ScaWeakProcessorRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScaWeakProcessorRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScaWeakProcessorRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        unsafe {
            ((*UPCALLS).scan_weak_processor_roots)(to_edges_closure_weakref::<_, _>(
                &mut self.factory,
            ));
        }
    }
}
