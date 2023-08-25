use crate::gc_work::*;
use crate::Edge;
use crate::{EdgesClosure, OpenJDK};
use crate::{NewBuffer, OpenJDKEdge, UPCALLS};
use mmtk::memory_manager;
use mmtk::scheduler::RootKind;
use mmtk::scheduler::WorkBucketStage;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::{EdgeVisitor, RootsWorkFactory, Scanning};
use mmtk::Mutator;
use mmtk::MutatorContext;

pub struct VMScanning {}

extern "C" fn report_edges_and_renew_buffer<E: Edge, F: RootsWorkFactory<E>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
    factory_ptr: *mut libc::c_void,
) -> NewBuffer {
    if !ptr.is_null() {
        let ptr = ptr as *mut E;
        let buf = unsafe { Vec::<E>::from_raw_parts(ptr, length, capacity) };
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

pub(crate) fn to_edges_closure<E: Edge, F: RootsWorkFactory<E>>(factory: &mut F) -> EdgesClosure {
    EdgesClosure {
        func: report_edges_and_renew_buffer::<E, F>,
        data: factory as *mut F as *mut libc::c_void,
    }
}

impl<const COMPRESSED: bool> Scanning<OpenJDK<COMPRESSED>> for VMScanning {
    fn scan_object(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut impl EdgeVisitor<OpenJDKEdge<COMPRESSED>>,
    ) {
        crate::object_scanning::scan_object::<_, _, COMPRESSED>(object, edge_visitor, tls);
    }

    fn scan_object_with_klass(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut impl EdgeVisitor<OpenJDKEdge<COMPRESSED>>,
        klass: Address,
    ) {
        crate::object_scanning::scan_object_with_klass::<_, _, COMPRESSED>(
            object,
            edge_visitor,
            tls,
            klass,
        );
    }

    fn obj_array_data(o: ObjectReference) -> crate::OpenJDKEdgeRange<COMPRESSED> {
        crate::object_scanning::obj_array_data::<COMPRESSED>(unsafe { std::mem::transmute(o) })
    }

    fn is_obj_array(o: ObjectReference) -> bool {
        crate::object_scanning::is_obj_array::<COMPRESSED>(unsafe { std::mem::transmute(o) })
    }

    fn is_val_array(o: ObjectReference) -> bool {
        crate::object_scanning::is_val_array::<COMPRESSED>(unsafe { std::mem::transmute(o) })
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: VMWorkerThread) {
        // unimplemented!()
        // TODO
    }

    fn scan_roots_in_mutator_thread(
        _tls: VMWorkerThread,
        mutator: &'static mut Mutator<OpenJDK<COMPRESSED>>,
        mut factory: impl RootsWorkFactory<OpenJDKEdge<COMPRESSED>>,
    ) {
        let tls = mutator.get_tls();
        unsafe {
            ((*UPCALLS).scan_roots_in_mutator_thread)(to_edges_closure(&mut factory), tls);
        }
    }

    fn scan_multiple_thread_root(
        _tls: VMWorkerThread,
        mutators: Vec<VMMutatorThread>,
        mut factory: impl RootsWorkFactory<<OpenJDK<COMPRESSED> as mmtk::vm::VMBinding>::VMEdge>,
    ) {
        // let t = if cfg!(feature = "roots_breakdown") {
        //     Some(std::time::SystemTime::now())
        // } else {
        //     None
        // };
        let len = mutators.len();
        let ptr = mutators.as_ptr();
        unsafe {
            ((*UPCALLS).scan_multiple_thread_roots)(
                to_edges_closure(&mut factory),
                std::mem::transmute(ptr),
                len,
            );
        }
        // if cfg!(feature = "roots_breakdown") {
        //     let ms = t.unwrap().elapsed().unwrap().as_micros() as f32 / 1000f32;
        //     eprintln!(" - ScanThreadRoots ({:.3}ms)", ms);
        // }
    }

    fn scan_vm_specific_roots(
        _tls: VMWorkerThread,
        factory: impl RootsWorkFactory<OpenJDKEdge<COMPRESSED>>,
    ) {
        let mut w = vec![
            Box::new(ScanUniverseRoots::new(factory.clone())) as _,
            Box::new(ScanJNIHandlesRoots::new(factory.clone())) as _,
            Box::new(ScanObjectSynchronizerRoots::new(factory.clone())) as _,
            Box::new(ScanManagementRoots::new(factory.clone())) as _,
            Box::new(ScanJvmtiExportRoots::new(factory.clone())) as _,
            Box::new(ScanAOTLoaderRoots::new(factory.clone())) as _,
            Box::new(ScanSystemDictionaryRoots::new(factory.clone())) as _,
            Box::new(ScanCodeCacheRoots::new(factory.clone())) as _,
            Box::new(ScanClassLoaderDataGraphRoots::new(factory.clone())) as _,
        ];
        if crate::singleton::<COMPRESSED>()
            .get_plan()
            .requires_weak_root_scanning()
        {
            w.push(Box::new(ScanNewWeakHandleRoots::new(factory.clone())) as _);
        }
        memory_manager::add_work_packets(
            &crate::singleton::<COMPRESSED>(),
            WorkBucketStage::RCProcessIncs,
            w,
        );
        memory_manager::add_work_packet(
            &crate::singleton::<COMPRESSED>(),
            WorkBucketStage::RCProcessIncs,
            ScanVMThreadRoots::new(factory),
        );
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
