use super::{OpenJDK, UPCALLS};
use crate::NewBuffer;
use crate::scanning::create_process_edges_work;
use mmtk::scheduler::*;
use mmtk::MMTK;
use mmtk::util::Address;
use std::slice;
use std::marker::PhantomData;
use std::sync::atomic::Ordering;


pub(crate) extern "C" fn create_process_edges_work_uni<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::UNIVERSE_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanUniverseRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanUniverseRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanUniverseRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_universe_roots)(create_process_edges_work_uni::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_jni<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::JNI_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanJNIHandlesRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanJNIHandlesRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanJNIHandlesRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_jni_handle_roots)(create_process_edges_work_jni::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_osync<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::OBJ_SYNC_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanObjectSynchronizerRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanObjectSynchronizerRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanObjectSynchronizerRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_object_synchronizer_roots)(create_process_edges_work_osync::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_mgmt<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::MGMT_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanManagementRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanManagementRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanManagementRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_management_roots)(create_process_edges_work_mgmt::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_jvmti<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::JVMTI_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanJvmtiExportRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanJvmtiExportRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanJvmtiExportRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_jvmti_export_roots)(create_process_edges_work_jvmti::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_aot<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::AOT_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanAOTLoaderRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanAOTLoaderRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanAOTLoaderRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_aot_loader_roots)(create_process_edges_work_aot::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_dict<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::SYS_DICT_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanSystemDictionaryRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanSystemDictionaryRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanSystemDictionaryRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_system_dictionary_roots)(create_process_edges_work_dict::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_code<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::CODE_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanCodeCacheRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanCodeCacheRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanCodeCacheRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_code_cache_roots)(create_process_edges_work_code::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_str<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::STR_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanStringTableRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanStringTableRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanStringTableRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_string_table_roots)(create_process_edges_work_str::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_cl<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::CL_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
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
            ((*UPCALLS).scan_class_loader_data_graph_roots)(create_process_edges_work_cl::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_weak<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::WEAK_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
}

pub struct ScanWeakProcessorRoots<E: ProcessEdgesWork<VM = OpenJDK>>(PhantomData<E>);

impl<E: ProcessEdgesWork<VM = OpenJDK>> ScanWeakProcessorRoots<E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ScanWeakProcessorRoots<E> {
    fn do_work(&mut self, _worker: &mut GCWorker<OpenJDK>, _mmtk: &'static MMTK<OpenJDK>) {
        unsafe {
            ((*UPCALLS).scan_weak_processor_roots)(create_process_edges_work_weak::<E> as _);
        }
    }
}

pub(crate) extern "C" fn create_process_edges_work_vm<W: ProcessEdgesWork<VM = OpenJDK>>(
    ptr: *mut Address,
    length: usize,
    capacity: usize,
) -> NewBuffer {
    if !ptr.is_null() {
        mmtk::VMT_ROOTS.fetch_add(length, Ordering::SeqCst);
    }
    create_process_edges_work::<W>(ptr, length, capacity)
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
            ((*UPCALLS).scan_vm_thread_roots)(create_process_edges_work_vm::<E> as _);
        }
    }
}
