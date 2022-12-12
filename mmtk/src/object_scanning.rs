use crate::reference_glue::DISCOVERED_LISTS;
use crate::SINGLETON;

use super::abi::*;
use super::{OpenJDKEdge, UPCALLS};
use mmtk::util::constants::*;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::EdgeVisitor;
use std::cell::UnsafeCell;
use std::{mem, slice};
trait OopIterate: Sized {
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>);
}

impl OopIterate for OopMapBlock {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        let start = oop.get_field_address(self.offset);
        for i in 0..self.count as usize {
            let edge = start + (i << LOG_BYTES_IN_ADDRESS);
            closure.visit_edge(edge);
        }
    }
}

impl OopIterate for InstanceKlass {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        if closure.should_follow_clds() {
            do_klass(&oop.klass, closure);
        }
        let oop_maps = self.nonstatic_oop_maps();
        for map in oop_maps {
            map.oop_iterate(oop, closure)
        }
    }
}

impl OopIterate for InstanceMirrorKlass {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        self.instance_klass.oop_iterate(oop, closure);
        if closure.should_follow_clds() {
            let klass = unsafe {
                (oop.start() + *crate::JAVA_LANG_CLASS_KLASS_OFFSET_IN_BYTES).load::<*mut Klass>()
            };
            if !klass.is_null() {
                let klass = unsafe { &mut *klass };
                if klass.is_instance_klass()
                    && (unsafe { klass.cast::<InstanceKlass>().is_anonymous() })
                {
                    do_cld(&klass.class_loader_data, closure)
                } else {
                    do_klass(klass, closure)
                }
            }
        }

        // static fields
        let start: *const Oop = Self::start_of_static_fields(oop).to_ptr::<Oop>();
        let len = Self::static_oop_field_count(oop);
        let slice = unsafe { slice::from_raw_parts(start, len as _) };
        for oop in slice {
            closure.visit_edge(Address::from_ref(oop as &Oop));
        }
    }
}

impl OopIterate for InstanceClassLoaderKlass {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        self.instance_klass.oop_iterate(oop, closure);
        if closure.should_follow_clds() {
            let cld = unsafe {
                (oop.start() + *crate::JAVA_LANG_CLASSLOADER_LOADER_DATA_OFFSET)
                    .load::<*mut ClassLoaderData>()
            };
            if !cld.is_null() {
                do_cld(unsafe { &*cld }, closure);
            }
        }
    }
}

impl OopIterate for ObjArrayKlass {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        if closure.should_follow_clds() {
            do_klass(&oop.klass, closure);
        }
        let array = unsafe { oop.as_array_oop() };
        for oop in unsafe { array.data::<Oop>(BasicType::T_OBJECT) } {
            closure.visit_edge(Address::from_ref(oop as &Oop));
        }
    }
}

impl OopIterate for TypeArrayKlass {
    #[inline(always)]
    fn oop_iterate(&self, _oop: Oop, _closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        // Performance tweak: We skip processing the klass pointer since all
        // TypeArrayKlasses are guaranteed processed via the null class loader.
    }
}

impl OopIterate for InstanceRefKlass {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        use crate::abi::*;
        self.instance_klass.oop_iterate(oop, closure);

        let disable_discovery = !closure.should_discover_references();

        if Self::should_discover_refs(self.instance_klass.reference_type, disable_discovery) {
            match self.instance_klass.reference_type {
                ReferenceType::None => {
                    panic!("oop_iterate on InstanceRefKlass with reference_type as None")
                }
                rt => {
                    if !Self::discover_reference(oop, rt) {
                        Self::process_ref_as_strong(oop, closure)
                    }
                }
            }
        } else {
            Self::process_ref_as_strong(oop, closure);
        }
    }
}

impl InstanceRefKlass {
    #[inline]
    fn should_discover_refs(mut rt: ReferenceType, disable_discovery: bool) -> bool {
        if rt == ReferenceType::Other {
            rt = ReferenceType::Weak;
        }
        if disable_discovery {
            return false;
        }
        if *SINGLETON.get_options().no_finalizer && rt == ReferenceType::Final {
            return false;
        }
        if *SINGLETON.get_options().no_reference_types && rt != ReferenceType::Final {
            return false;
        }
        true
    }
    #[inline]
    fn process_ref_as_strong(oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
        let referent_addr = Self::referent_address(oop);
        closure.visit_edge(referent_addr);
        let discovered_addr = Self::discovered_address(oop);
        closure.visit_edge(discovered_addr);
    }
    #[inline]
    fn discover_reference(oop: Oop, rt: ReferenceType) -> bool {
        // Do not discover new refs during reference processing.
        if !DISCOVERED_LISTS.allow_discover() {
            return false;
        }
        // Do not discover if the referent is live.
        let referent: ObjectReference = unsafe { InstanceRefKlass::referent_address(oop).load() };
        // Skip live or null referents
        if referent.is_reachable() || referent.is_null() {
            return false;
        }
        // Skip young referents
        let reference: ObjectReference = oop.into();
        if !SINGLETON
            .get_plan()
            .should_process_reference(reference, referent)
        {
            return false;
        }
        // Add to reference list
        DISCOVERED_LISTS.get(rt).add(reference, referent);
        true
    }
}

#[allow(unused)]
fn oop_iterate_slow<V: EdgeVisitor<OpenJDKEdge>>(oop: Oop, closure: &mut V, tls: OpaquePointer) {
    unsafe {
        CLOSURE.with(|x| *x.get() = closure as *mut V as *mut u8);
        ((*UPCALLS).scan_object)(
            mem::transmute(scan_object_fn::<V> as *const unsafe extern "C" fn(edge: Address)),
            mem::transmute(oop),
            tls,
            closure.should_follow_clds(),
            closure.should_claim_clds(),
        );
    }
}

#[inline(always)]
fn oop_iterate<V: EdgeVisitor<OpenJDKEdge>>(oop: Oop, closure: &mut V) {
    unsafe {
        CLOSURE.with(|x| *x.get() = closure as *mut V as *mut u8);
    }
    let klass_id = oop.klass.id;
    debug_assert!(
        klass_id as i32 >= 0 && (klass_id as i32) < 6,
        "Invalid klass-id: {:x} for oop: {:x}",
        klass_id as i32,
        unsafe { mem::transmute::<Oop, ObjectReference>(oop) }
    );
    match klass_id {
        KlassID::Instance => {
            let instance_klass = unsafe { oop.klass.cast::<InstanceKlass>() };
            instance_klass.oop_iterate(oop, closure);
        }
        KlassID::InstanceClassLoader => {
            let instance_klass = unsafe { oop.klass.cast::<InstanceClassLoaderKlass>() };
            instance_klass.oop_iterate(oop, closure);
        }
        KlassID::InstanceMirror => {
            let instance_klass = unsafe { oop.klass.cast::<InstanceMirrorKlass>() };
            instance_klass.oop_iterate(oop, closure);
        }
        KlassID::ObjArray => {
            let array_klass = unsafe { oop.klass.cast::<ObjArrayKlass>() };
            array_klass.oop_iterate(oop, closure);
        }
        // KlassID::TypeArray => {
        //     // let array_klass = unsafe { oop.klass.cast::<TypeArrayKlass>() };
        //     // array_klass.oop_iterate(oop, closure);
        // }
        KlassID::InstanceRef => {
            let instance_klass = unsafe { oop.klass.cast::<InstanceRefKlass>() };
            instance_klass.oop_iterate(oop, closure);
        }
        // _ => oop_iterate_slow(oop, closure, OpaquePointer::UNINITIALIZED),
        _ => {}
    }
}

fn do_cld<V: EdgeVisitor<OpenJDKEdge>>(cld: &ClassLoaderData, closure: &mut V) {
    if !closure.should_follow_clds() {
        return;
    }
    cld.oops_do(closure)
}

fn do_klass<V: EdgeVisitor<OpenJDKEdge>>(klass: &Klass, closure: &mut V) {
    if !closure.should_follow_clds() {
        return;
    }
    do_cld(&klass.class_loader_data, closure)
}

#[inline(always)]
pub fn is_obj_array(oop: Oop) -> bool {
    oop.klass.id == KlassID::ObjArray
}

#[inline(always)]
pub fn is_val_array(oop: Oop) -> bool {
    oop.klass.id == KlassID::TypeArray
}

#[inline(always)]
pub fn obj_array_data(oop: Oop) -> &'static [ObjectReference] {
    unsafe {
        let array = oop.as_array_oop();
        array.data::<ObjectReference>(BasicType::T_OBJECT)
    }
}

thread_local! {
    static CLOSURE: UnsafeCell<*mut u8> = UnsafeCell::new(std::ptr::null_mut());
}

pub unsafe extern "C" fn scan_object_fn<V: EdgeVisitor<OpenJDKEdge>>(edge: Address) {
    let ptr: *mut u8 = CLOSURE.with(|x| *x.get());
    let closure = &mut *(ptr as *mut V);
    closure.visit_edge(edge);
}

#[inline]
pub fn scan_object(
    object: ObjectReference,
    closure: &mut impl EdgeVisitor<OpenJDKEdge>,
    _tls: VMWorkerThread,
) {
    // println!("*****scan_object(0x{:x}) -> \n 0x{:x}, 0x{:x} \n",
    //     object,
    //     unsafe { *(object.value() as *const usize) },
    //     unsafe { *((object.value() + 8) as *const usize) }
    // );
    unsafe { oop_iterate(mem::transmute(object), closure) }
}
