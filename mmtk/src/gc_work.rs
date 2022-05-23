use super::{OpenJDK, UPCALLS};
use crate::scanning::{create_process_edges_work, create_process_edges_work_vec};
use mmtk::scheduler::*;
use mmtk::MMTK;
use std::marker::PhantomData;
use std::sync::atomic::Ordering;

pub struct ScanCodeCacheRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanCodeCacheRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanCodeCacheRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        let mut vec = Vec::with_capacity(crate::TOTAL_SIZE.load(Ordering::Relaxed));
        for (_, roots) in &*crate::CODE_CACHE_ROOTS.lock() {
            for r in roots {
                vec.push(*r)
            }
        }
        create_process_edges_work_vec::<E>(vec)
    }
}

pub struct ScanClassLoaderDataGraphRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanClassLoaderDataGraphRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanClassLoaderDataGraphRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_class_loader_data_graph_roots)(create_process_edges_work::<E> as _);
        }
    }
}

pub struct ScanOopStorageSetRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanOopStorageSetRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanOopStorageSetRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_oop_storage_set_roots)(create_process_edges_work::<E> as _);
        }
    }
}

pub struct ScanVMThreadRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanVMThreadRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanVMThreadRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_vm_thread_roots)(create_process_edges_work::<E> as _);
        }
    }
}
