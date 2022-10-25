use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

use crate::abi::{InstanceRefKlass, Oop, ReferenceType};
use crate::OpenJDK;
use atomic::{Atomic, Ordering};
use mmtk::scheduler::{GCWork, GCWorker, ProcessEdgesWork, WorkBucketStage};
use mmtk::util::opaque_pointer::VMWorkerThread;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::ReferenceGlue;
use mmtk::MMTK;

#[inline(always)]
pub fn set_referent(reff: ObjectReference, referent: ObjectReference) {
    let oop = Oop::from(reff);
    unsafe { InstanceRefKlass::referent_address(oop).store(referent) };
}

#[inline(always)]
fn get_referent(object: ObjectReference) -> ObjectReference {
    let oop = Oop::from(object);
    unsafe { InstanceRefKlass::referent_address(oop).load::<ObjectReference>() }
}

#[inline(always)]
fn get_next_reference_slot(object: ObjectReference) -> Address {
    let oop = Oop::from(object);
    unsafe { InstanceRefKlass::discovered_address(oop) }
}

#[inline(always)]
fn get_next_reference(object: ObjectReference) -> ObjectReference {
    unsafe { get_next_reference_slot(object).load() }
}

#[inline(always)]
fn set_next_reference(object: ObjectReference, next: ObjectReference) {
    unsafe { get_next_reference_slot(object).store(next) }
}

pub struct VMReferenceGlue {}

impl ReferenceGlue<OpenJDK> for VMReferenceGlue {
    type FinalizableType = ObjectReference;

    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        let oop = Oop::from(reff);
        unsafe { InstanceRefKlass::referent_address(oop).store(referent) };
    }
    fn get_referent(object: ObjectReference) -> ObjectReference {
        let oop = Oop::from(object);
        unsafe { InstanceRefKlass::referent_address(oop).load::<ObjectReference>() }
    }
    fn enqueue_references(_references: &[ObjectReference], _tls: VMWorkerThread) {
        // unsafe {
        //     ((*UPCALLS).enqueue_references)(references.as_ptr(), references.len());
        // }
    }
}

pub struct DiscoveredList {
    head: Atomic<ObjectReference>,
    rt: ReferenceType,
}

impl DiscoveredList {
    fn new(rt: ReferenceType) -> Self {
        Self {
            head: Atomic::new(ObjectReference::NULL),
            rt,
        }
    }

    #[inline]
    pub fn add(&self, obj: ObjectReference) {
        // let _g = LOCK.lock().unwrap();
        let oop = Oop::from(obj);
        let head = self.head.load(Ordering::SeqCst);
        let discovered_addr = unsafe {
            InstanceRefKlass::discovered_address(oop).as_ref::<Atomic<ObjectReference>>()
        };
        let next_discovered = if head.is_null() { obj } else { head };
        debug_assert!(!next_discovered.is_null());
        if discovered_addr
            .compare_exchange(
                ObjectReference::NULL,
                next_discovered,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            // println!("add {:?} {:?}", self.rt, obj);
            self.head.store(obj, Ordering::SeqCst);
        }
        debug_assert!(!get_next_reference(obj).is_null());
    }
}

pub struct DiscoveredLists {
    pub soft: Vec<DiscoveredList>,
    pub weak: Vec<DiscoveredList>,
    pub phantom: Vec<DiscoveredList>,
    allow_discover: AtomicBool,
}

impl DiscoveredLists {
    pub fn new() -> Self {
        let workers = crate::SINGLETON.scheduler.num_workers();
        let build_lists = |rt| (0..workers).map(|_| DiscoveredList::new(rt)).collect();
        Self {
            soft: build_lists(ReferenceType::Soft),
            weak: build_lists(ReferenceType::Weak),
            phantom: build_lists(ReferenceType::Phantom),
            allow_discover: AtomicBool::new(true),
        }
    }

    #[inline]
    pub fn enable_discover(&self) {
        self.allow_discover.store(true, Ordering::SeqCst)
    }

    #[inline(always)]
    pub fn allow_discover(&self) -> bool {
        self.allow_discover.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn get_by_rt_and_index(&self, rt: ReferenceType, index: usize) -> &DiscoveredList {
        match rt {
            ReferenceType::Soft => &self.soft[index],
            ReferenceType::Weak => &self.weak[index],
            ReferenceType::Phantom => &self.phantom[index],
            _ => unimplemented!(),
        }
    }

    #[inline]
    pub fn get(&self, rt: ReferenceType) -> &DiscoveredList {
        let id = mmtk::scheduler::worker::current_worker_ordinal().unwrap();
        self.get_by_rt_and_index(rt, id)
    }

    pub fn process_lists<E: ProcessEdgesWork<VM = OpenJDK>>(
        &self,
        worker: &mut GCWorker<OpenJDK>,
        rt: ReferenceType,
        lists: &[DiscoveredList],
    ) {
        let mut packets = vec![];
        for i in 0..lists.len() {
            let head = lists[i].head.load(Ordering::SeqCst);
            if head.is_null() {
                continue;
            }
            let w = ProcessDiscoveredList {
                list_index: i,
                head,
                rt,
                _p: PhantomData::<E>,
            };
            packets.push(Box::new(w) as Box<dyn GCWork<OpenJDK>>);
        }
        worker.scheduler().work_buckets[WorkBucketStage::Unconstrained].bulk_add(packets);
    }

    pub fn process<E: ProcessEdgesWork<VM = OpenJDK>>(&self, worker: &mut GCWorker<OpenJDK>) {
        println!(
            "process refs {}",
            worker.mmtk.plan.is_emergency_collection()
        );
        self.allow_discover.store(false, Ordering::SeqCst);
        self.process_lists::<E>(worker, ReferenceType::Soft, &self.soft);
        self.process_lists::<E>(worker, ReferenceType::Weak, &self.weak);
        self.process_lists::<E>(worker, ReferenceType::Phantom, &self.phantom);
    }
}

lazy_static! {
    pub static ref LOCK: Mutex<()> = Mutex::new(());
    pub static ref DISCOVERED_LISTS: DiscoveredLists = DiscoveredLists::new();
}

pub struct ProcessDiscoveredList<E: ProcessEdgesWork<VM = OpenJDK>> {
    list_index: usize,
    head: ObjectReference,
    rt: ReferenceType,
    _p: PhantomData<E>,
}

impl<E: ProcessEdgesWork<VM = OpenJDK>> GCWork<OpenJDK> for ProcessDiscoveredList<E> {
    fn do_work(&mut self, worker: &mut GCWorker<OpenJDK>, mmtk: &'static MMTK<OpenJDK>) {
        let mut trace = E::new(vec![], false, mmtk);
        trace.set_worker(worker);
        // mmtk.reference_processors.scan_soft_refs(&mut w, mmtk);
        let retain = true || self.rt == ReferenceType::Soft && !mmtk.plan.is_emergency_collection();
        // if rt == ReferenceType::Soft && !mmtk.plan.is_emergency_collection() {
        //     // Retain soft refs
        //     let mut obj = self.head;
        //     let mut prev_obj = ObjectReference::NULL;
        //     while obj != prev_obj {
        //         if !reference.is_live() {}
        //         prev_obj = obj;
        //         obj = get_next_reference(obj);
        //     }
        // }
        let mut cursor = &mut self.head;
        let mut holder: Option<ObjectReference> = None;
        loop {
            debug_assert!(!cursor.is_null());
            let mut reference = *cursor;
            // Remove reference from list if it is dead
            if !reference.is_live() {
                set_referent(reference, ObjectReference::NULL);
                let next_ref = get_next_reference(reference);
                if next_ref == reference {
                    if let Some(holder) = holder {
                        *cursor = holder;
                    } else {
                        *cursor = ObjectReference::NULL;
                    }
                    break;
                } else {
                    debug_assert!(!next_ref.is_null());
                    *cursor = next_ref;
                    debug_assert!(!cursor.is_null());
                    continue;
                }
            } else {
                let new_reference = reference.get_forwarded_object().unwrap_or(reference);
                // println!("reference {:?} => {:?}", reference, new_reference);
                reference = new_reference;
                *cursor = reference;
            }
            let referent = get_referent(reference);
            if !referent.is_null() && (referent.is_live() || retain) {
                // Keep this ref
                let forwarded = trace.trace_object(referent);
                set_referent(reference, forwarded);
            } else {
                // Clear this ref
                set_referent(reference, ObjectReference::NULL);
            }
            cursor = unsafe { &mut *get_next_reference_slot(reference).to_mut_ptr() };
            holder = Some(reference);
            let next_ref = get_next_reference(reference);
            if next_ref.is_live() {
                let next_ref = next_ref.get_forwarded_object().unwrap_or(next_ref);
                if next_ref == reference {
                    set_next_reference(reference, next_ref);
                    break;
                }
            }
            debug_assert!(!cursor.is_null(), "{:?}", reference);
        }
        trace.flush();
        DISCOVERED_LISTS
            .get_by_rt_and_index(self.rt, self.list_index)
            .head
            .store(self.head, Ordering::SeqCst);
    }
}
