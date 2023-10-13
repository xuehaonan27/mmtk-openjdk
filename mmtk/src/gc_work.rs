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
        unsafe {
            ((*UPCALLS).scan_class_loader_data_graph_roots)(
                to_edges_closure::<VM::VMEdge, F>(&mut self.factory),
                to_edges_closure::<VM::VMEdge, F>(&mut self.factory),
                mmtk.get_plan()
                    .current_gc_should_scan_all_classloader_strong_roots(),
            );
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
        // let t = if cfg!(feature = "roots_breakdown") {
        //     Some(std::time::SystemTime::now())
        // } else {
        //     None
        // };
        // let mut new_roots = crate::NURSERY_WEAK_HANDLE_ROOTS.lock().unwrap();
        // if cfg!(feature = "roots_breakdown") {
        //     record_roots(new_roots.len());
        // }
        // for slice in new_roots.chunks(mmtk::args::BUFFER_SIZE) {
        //     let slice = unsafe { std::mem::transmute::<&[Address], &[VM::VMEdge]>(slice) };
        //     println!("Weak Handle Count={}", slice.len());
        //     self.factory
        //         .create_process_edge_roots_work(slice.to_vec(), RootKind::Strong);
        // }
        // new_roots.clear();
        // if cfg!(feature = "roots_breakdown") {
        //     let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
        //     report_roots("NewWeakHandleRoots", ms);
        // }
        // unsafe {
        //     ((*UPCALLS).scan_weak_processor_roots)(
        //         to_edges_closure_cld::<VM::VMEdge, F, false>(&mut self.factory),
        //         _mmtk
        //             .get_plan()
        //             .current_gc_should_scan_all_classloader_strong_roots(),
        //     );
        // }

        unsafe {
            ((*UPCALLS).scan_weak_processor_roots)(
                to_edges_closure2::<VM::VMEdge, F>(&mut self.factory),
                false,
            );
        }

        // unsafe {
        //     ((*UPCALLS).scams)(
        //         to_edges_closure2::<VM::VMEdge, F>(&mut self.factory),
        //         false,
        //     );
        // }
    }
}
extern "C" fn report_edges_and_renew_buffer2<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        // Note: Currently OpenJDKEdge has the same layout as Address.  If the layout changes, we
        // should fix the Rust-to-C interface.
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr as _, length, capacity) };
        // for e in &buf {
        // println!("{:?} -> {:?}", e.to_address(), e.load());
        // }
        if cfg!(feature = "roots_breakdown") {
            super::gc_work::record_roots(buf.len());
        }
        let factory: &mut F = unsafe { &mut *(factory_ptr as *mut F) };
        factory.create_process_edge_roots_work(buf, RootKind::Strong);
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

pub(crate) fn to_edges_closure2<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer2::<E, F>,
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
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        unsafe {
            ((*UPCALLS).scan_code_cache_roots)(to_edges_closure::<VM::VMEdge, F>(
                &mut self.factory,
            ));
        }
    }
}
