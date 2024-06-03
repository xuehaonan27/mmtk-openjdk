use crate::scanning::to_edges_closure;
use crate::Address;
use crate::Edge;
use crate::EdgesClosure;
use crate::NewBuffer;
use crate::UPCALLS;
use mmtk::scheduler::*;
use mmtk::vm::RootsWorkFactory;
use mmtk::vm::*;
use mmtk::MMTK;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    pub static COUNT: AtomicUsize = AtomicUsize::new(0);
}

pub fn record_roots(len: usize) {
    super::gc_work::COUNT.with(|x| {
        let c = x.load(Ordering::Relaxed);
        x.store(c + len, Ordering::Relaxed);
    });
}

fn report_roots(name: &str, ms: f32) {
    super::gc_work::COUNT.with(|x| {
        let c = x.load(Ordering::Relaxed);
        eprintln!(" - {} roots count: {} ({:.3}ms)", name, c, ms);
        x.store(0, Ordering::Relaxed);
    });
}

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
                let t = if cfg!(feature = "roots_breakdown") {
                    Some(std::time::SystemTime::now())
                } else {
                    None
                };
                unsafe {
                    ((*UPCALLS).$func_name)(to_edges_closure(&mut self.factory));
                }
                if cfg!(feature = "roots_breakdown") {
                    let name = stringify!($struct_name);
                    let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
                    report_roots(&name[4..name.len() - 5], ms);
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
scan_roots_work!(ScanVMThreadRoots, scan_vm_thread_roots);

extern "C" fn report_edges_and_renew_buffer_cld<
    E: Edge,
    F: RootsWorkFactory<E>,
    const WEAK: bool,
    const ALL_STRONG: bool,
>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
        if cfg!(feature = "roots_breakdown") {
            record_roots(buf.len());
        }
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        let kind = if WEAK {
            RootKind::YoungWeakCLDRoots
        } else if ALL_STRONG {
            RootKind::StrongCLDRoots
        } else {
            RootKind::YoungStrongCLDRoots
        };
        factory.create_process_edge_roots_work(buf, kind);
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

fn to_edges_closure_cld<
    E: Edge,
    F: RootsWorkFactory<E>,
    const WEAK: bool,
    const ALL_STRONG: bool,
>(
    factory: &mut F,
) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_cld::<E, F, WEAK, ALL_STRONG>,
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
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let scan_all_strong_roots = mmtk.get_plan().current_gc_should_perform_class_unloading();
        if scan_all_strong_roots {
            unsafe {
                ((*UPCALLS).scan_class_loader_data_graph_roots)(
                    to_edges_closure_cld::<VM::VMEdge, F, false, true>(&mut self.factory),
                    to_edges_closure_cld::<VM::VMEdge, F, true, false>(&mut self.factory),
                    scan_all_strong_roots,
                );
            }
        } else {
            unsafe {
                ((*UPCALLS).scan_class_loader_data_graph_roots)(
                    to_edges_closure_cld::<VM::VMEdge, F, false, false>(&mut self.factory),
                    to_edges_closure_cld::<VM::VMEdge, F, true, false>(&mut self.factory),
                    scan_all_strong_roots,
                );
            }
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            report_roots("ClassLoaderDataGraph", ms);
        }
    }
}

pub struct ScanNewWeakHandleRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanNewWeakHandleRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanNewWeakHandleRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let mut new_roots = crate::NURSERY_WEAK_HANDLE_ROOTS.lock().unwrap();
        if cfg!(feature = "roots_breakdown") {
            record_roots(new_roots.len());
        }
        for slice in new_roots.chunks(mmtk::args::BUFFER_SIZE) {
            let slice = unsafe { std::mem::transmute::<&[Address], &[VM::VMEdge]>(slice) };
            self.factory
                .create_process_edge_roots_work(slice.to_vec(), RootKind::YoungWeakHandleRoots);
        }
        new_roots.clear();
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            report_roots("NewWeakHandleRoots", ms);
        }
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
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let mut edges = Vec::with_capacity(F::BUFFER_SIZE);
        let mut mature = crate::MATURE_CODE_CACHE_ROOTS.lock().unwrap();
        let mut nursery_guard = crate::NURSERY_CODE_CACHE_ROOTS.lock().unwrap();
        let nursery = std::mem::take::<HashMap<Address, Vec<Address>>>(&mut nursery_guard);
        let mut c = 0;
        // Young roots
        for (key, roots) in nursery {
            for r in &roots {
                edges.push(VM::VMEdge::from_address(*r));
                if edges.len() >= F::BUFFER_SIZE {
                    if cfg!(feature = "roots_breakdown") {
                        c += edges.len();
                    }
                    self.factory.create_process_edge_roots_work(
                        std::mem::take(&mut edges),
                        RootKind::YoungCodeCacheRoots,
                    );
                    edges.reserve(F::BUFFER_SIZE);
                }
            }
            mature.insert(key, roots);
        }
        if !edges.is_empty() {
            if cfg!(feature = "roots_breakdown") {
                c += edges.len();
            }
            self.factory
                .create_process_edge_roots_work(edges, RootKind::YoungCodeCacheRoots);
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            eprintln!(" - NewCodeCache roots count: {} ({:.3})", c, ms);
        }
    }
}

extern "C" fn report_edges_and_renew_buffer_weak<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
        if cfg!(feature = "roots_breakdown") {
            record_roots(buf.len());
        }
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        let kind = RootKind::Weak;
        factory.create_process_edge_roots_work(buf, kind);
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

fn to_edges_closure_weak<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_weak::<E, F>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

pub struct ScanWeakStringTableRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanWeakStringTableRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanWeakStringTableRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let scan_all_strong_roots = mmtk.get_plan().current_gc_should_perform_class_unloading();
        assert!(scan_all_strong_roots);
        unsafe {
            ((*UPCALLS).scan_string_table_roots)(
                to_edges_closure_weak::<VM::VMEdge, F>(&mut self.factory),
                false,
            );
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            report_roots("WeakStringTableRoots", ms);
        }
    }
}

#[allow(unused)]
pub struct ScanWeakProcessorRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanWeakProcessorRoots<E, F> {
    #[allow(unused)]
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanWeakProcessorRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let scan_all_strong_roots = mmtk.get_plan().current_gc_should_perform_class_unloading();
        assert!(scan_all_strong_roots);
        unsafe {
            ((*UPCALLS).scan_weak_processor_roots)(
                to_edges_closure_weak::<VM::VMEdge, F>(&mut self.factory),
                false,
            );
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            report_roots("WeakProcessorRoots", ms);
        }
    }
}

pub struct ScanWeakCodeCacheRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanWeakCodeCacheRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanWeakCodeCacheRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        let scan_all_strong_roots = mmtk.get_plan().current_gc_should_perform_class_unloading();
        assert!(scan_all_strong_roots);

        let mut edges = Vec::with_capacity(F::BUFFER_SIZE);
        let mature = crate::MATURE_CODE_CACHE_ROOTS.lock().unwrap();
        let mut c = 0;
        // Young roots
        for (_key, roots) in &*mature {
            for r in roots {
                edges.push(VM::VMEdge::from_address(*r));
                if edges.len() >= F::BUFFER_SIZE {
                    if cfg!(feature = "roots_breakdown") {
                        c += edges.len();
                    }
                    self.factory
                        .create_process_edge_roots_work(std::mem::take(&mut edges), RootKind::Weak);
                    edges.reserve(F::BUFFER_SIZE);
                }
            }
        }
        if !edges.is_empty() {
            if cfg!(feature = "roots_breakdown") {
                c += edges.len();
            }
            self.factory
                .create_process_edge_roots_work(edges, RootKind::Weak);
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            eprintln!(" - WeakodeCacheRoots roots count: {} ({:.3})", c, ms);
        }
    }
}

extern "C" fn report_edges_and_renew_buffer_st<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
        if cfg!(feature = "roots_breakdown") {
            record_roots(buf.len());
        }
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_edge_roots_work(buf, RootKind::Weak);
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

fn to_edges_closure_st<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer_st::<E, F>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

pub struct ScanStringTableRoots<E: Edge, F: RootsWorkFactory<E>> {
    factory: F,
    _p: PhantomData<E>,
}

impl<E: Edge, F: RootsWorkFactory<E>> ScanStringTableRoots<E, F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, F: RootsWorkFactory<VM::VMEdge>> GCWork<VM>
    for ScanStringTableRoots<VM::VMEdge, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let t = if cfg!(feature = "roots_breakdown") {
            Some(std::time::SystemTime::now())
        } else {
            None
        };
        unsafe {
            ((*UPCALLS).scan_string_table_roots)(
                to_edges_closure_st::<VM::VMEdge, F>(&mut self.factory),
                mmtk.get_plan()
                    .downcast_ref::<mmtk::plan::lxr::LXR<VM>>()
                    .is_some(),
            );
        }
        if cfg!(feature = "roots_breakdown") {
            let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
            report_roots("StringTable", ms);
        }
    }
}
