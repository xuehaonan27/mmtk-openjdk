use super::abi::*;
use super::UPCALLS;
use crate::reference_glue::DISCOVERED_LISTS;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::edge_shape::Edge;
use mmtk::vm::EdgeVisitor;
use std::cell::UnsafeCell;
use std::{mem, slice};

trait OopIterate: Sized {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    );
}

impl OopIterate for OopMapBlock {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        let log_bytes_in_oop = if COMPRESSED { 2 } else { 3 };
        let start = oop.get_field_address(self.offset);
        for i in 0..self.count as usize {
            let edge = (start + (i << log_bytes_in_oop)).into();
            closure.visit_edge(edge);
        }
    }
}

impl OopIterate for InstanceKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        if closure.should_follow_clds() {
            do_klass::<_, _, COMPRESSED>(oop.klass::<COMPRESSED>(), closure);
        }
        let oop_maps = self.nonstatic_oop_maps();
        for map in oop_maps {
            map.oop_iterate::<E, V, COMPRESSED>(oop, closure)
        }
    }
}

impl OopIterate for InstanceMirrorKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        self.instance_klass
            .oop_iterate::<_, _, COMPRESSED>(oop, closure);
        if closure.should_follow_clds() {
            let klass = unsafe {
                (oop.start() + *crate::JAVA_LANG_CLASS_KLASS_OFFSET_IN_BYTES).load::<*mut Klass>()
            };
            if !klass.is_null() {
                let klass = unsafe { &mut *klass };
                if klass.is_instance_klass()
                    && (unsafe { klass.cast::<InstanceKlass>().is_anonymous() })
                {
                    do_cld::<_, _, COMPRESSED>(&klass.class_loader_data, closure)
                } else {
                    do_klass::<_, _, COMPRESSED>(klass, closure)
                }
            }
        }

        // static fields
        let start = Self::start_of_static_fields(oop);
        let len = Self::static_oop_field_count(oop);
        if COMPRESSED {
            let start: *const NarrowOop = start.to_ptr::<NarrowOop>();
            let slice = unsafe { slice::from_raw_parts(start, len as _) };
            for narrow_oop in slice {
                closure.visit_edge(narrow_oop.slot().into());
            }
        } else {
            let start: *const Oop = start.to_ptr::<Oop>();
            let slice = unsafe { slice::from_raw_parts(start, len as _) };
            for oop in slice {
                closure.visit_edge(Address::from_ref(oop as &Oop).into());
            }
        }
    }
}

impl OopIterate for InstanceClassLoaderKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        self.instance_klass
            .oop_iterate::<_, _, COMPRESSED>(oop, closure);
        if closure.should_follow_clds() {
            let cld = unsafe {
                (oop.start() + *crate::JAVA_LANG_CLASSLOADER_LOADER_DATA_OFFSET)
                    .load::<*mut ClassLoaderData>()
            };
            if !cld.is_null() {
                do_cld::<_, _, COMPRESSED>(unsafe { &*cld }, closure);
            }
        }
    }
}

impl OopIterate for ObjArrayKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        if closure.should_follow_clds() {
            do_klass::<_, _, COMPRESSED>(oop.klass::<COMPRESSED>(), closure);
        }
        let array = unsafe { oop.as_array_oop() };
        if COMPRESSED {
            for narrow_oop in unsafe { array.data::<NarrowOop, COMPRESSED>(BasicType::T_OBJECT) } {
                closure.visit_edge(narrow_oop.slot().into());
            }
        } else {
            for oop in unsafe { array.data::<Oop, COMPRESSED>(BasicType::T_OBJECT) } {
                closure.visit_edge(Address::from_ref(oop as &Oop).into());
            }
        }
    }
}

impl OopIterate for TypeArrayKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        _oop: Oop,
        _closure: &mut V,
    ) {
        // Performance tweak: We skip processing the klass pointer since all
        // TypeArrayKlasses are guaranteed processed via the null class loader.
    }
}

impl OopIterate for InstanceRefKlass {
    fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
        &self,
        oop: Oop,
        closure: &mut V,
    ) {
        use crate::abi::*;
        self.instance_klass
            .oop_iterate::<_, _, COMPRESSED>(oop, closure);

        let disable_discovery = !closure.should_discover_references();

        if Self::should_discover_refs::<COMPRESSED>(
            self.instance_klass.reference_type,
            disable_discovery,
        ) {
            match self.instance_klass.reference_type {
                ReferenceType::None => {
                    panic!("oop_iterate on InstanceRefKlass with reference_type as None")
                }
                rt => {
                    if !Self::discover_reference::<E, COMPRESSED>(oop, rt) {
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
    fn should_discover_refs<const COMPRESSED: bool>(
        mut rt: ReferenceType,
        disable_discovery: bool,
    ) -> bool {
        if rt == ReferenceType::Other {
            rt = ReferenceType::Weak;
        }
        if disable_discovery {
            return false;
        }
        if *crate::singleton::<COMPRESSED>().get_options().no_finalizer
            && rt == ReferenceType::Final
        {
            return false;
        }
        if *crate::singleton::<COMPRESSED>()
            .get_options()
            .no_reference_types
            && rt != ReferenceType::Final
        {
            return false;
        }
        true
    }
    fn process_ref_as_strong<E: Edge, V: EdgeVisitor<E>>(oop: Oop, closure: &mut V) {
        let referent_addr = Self::referent_address::<E>(oop);
        closure.visit_edge(referent_addr);
        let discovered_addr = Self::discovered_address::<E>(oop);
        closure.visit_edge(discovered_addr);
    }
    fn discover_reference<E: Edge, const COMPRESSED: bool>(oop: Oop, rt: ReferenceType) -> bool {
        // Do not discover new refs during reference processing.
        if !DISCOVERED_LISTS.allow_discover() {
            return false;
        }
        // Do not discover if the referent is live.
        let addr = InstanceRefKlass::referent_address::<E>(oop);
        let referent: ObjectReference = addr.load();
        // Skip live or null referents
        if referent.is_reachable() || referent.is_null() {
            return false;
        }
        // Skip young referents
        let reference: ObjectReference = oop.into();
        if !crate::singleton::<COMPRESSED>()
            .get_plan()
            .should_process_reference(reference, referent)
        {
            return false;
        }
        if rt == ReferenceType::Final && DISCOVERED_LISTS.is_discovered::<E>(reference) {
            return false;
        }
        // Add to reference list
        DISCOVERED_LISTS
            .get(rt)
            .add::<E, COMPRESSED>(reference, referent);
        true
    }
}

#[allow(unused)]
fn oop_iterate_slow<E: Edge, V: EdgeVisitor<E>>(oop: Oop, closure: &mut V, tls: OpaquePointer) {
    unsafe {
        CLOSURE.with(|x| *x.get() = closure as *mut V as *mut u8);
        ((*UPCALLS).scan_object)(
            mem::transmute(scan_object_fn::<E, V> as *const unsafe extern "C" fn(edge: Address)),
            mem::transmute(oop),
            tls,
            closure.should_follow_clds(),
            closure.should_claim_clds(),
        );
    }
}

fn oop_iterate<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
    oop: Oop,
    closure: &mut V,
    klass: Option<Address>,
) {
    let klass = if let Some(klass) = klass {
        unsafe { &*(klass.as_usize() as *const Klass) }
    } else {
        oop.klass::<COMPRESSED>()
    };
    let klass_id = klass.id;
    assert!(
        klass_id as i32 >= 0 && (klass_id as i32) < 6,
        "Invalid klass-id: {:x} for oop: {:x}",
        klass_id as i32,
        unsafe { mem::transmute::<Oop, ObjectReference>(oop) }
    );
    match klass_id {
        KlassID::Instance => {
            let instance_klass = unsafe { klass.cast::<InstanceKlass>() };
            instance_klass.oop_iterate::<E, V, COMPRESSED>(oop, closure);
        }
        KlassID::InstanceClassLoader => {
            let instance_klass = unsafe { klass.cast::<InstanceClassLoaderKlass>() };
            instance_klass.oop_iterate::<E, V, COMPRESSED>(oop, closure);
        }
        KlassID::InstanceMirror => {
            let instance_klass = unsafe { klass.cast::<InstanceMirrorKlass>() };
            instance_klass.oop_iterate::<E, V, COMPRESSED>(oop, closure);
        }
        KlassID::ObjArray => {
            let array_klass = unsafe { klass.cast::<ObjArrayKlass>() };
            array_klass.oop_iterate::<E, V, COMPRESSED>(oop, closure);
        }
        KlassID::TypeArray => {
            //     let array_klass = unsafe { oop.klass::<COMPRESSED>().cast::<TypeArrayKlass>() };
            //     array_klass.oop_iterate::<C, COMPRESSED>(oop, closure);
        }
        KlassID::InstanceRef => {
            let instance_klass = unsafe { klass.cast::<InstanceRefKlass>() };
            instance_klass.oop_iterate::<E, V, COMPRESSED>(oop, closure);
        }
        #[allow(unreachable_patterns)]
        _ => unreachable!(), // _ => oop_iterate_slow(oop, closure, OpaquePointer::UNINITIALIZED),
    }
}

fn do_cld<E: Edge, V: EdgeVisitor<E>, const COMPRESSED: bool>(
    cld: &ClassLoaderData,
    closure: &mut V,
) {
    if !closure.should_follow_clds() {
        return;
    }
    cld.oops_do::<_, _, COMPRESSED>(closure)
}

fn do_klass<E: Edge, V: EdgeVisitor<E>, const COMPRESSED: bool>(klass: &Klass, closure: &mut V) {
    if !closure.should_follow_clds() {
        return;
    }
    do_cld::<_, _, COMPRESSED>(&klass.class_loader_data, closure)
}

pub fn is_obj_array<const COMPRESSED: bool>(oop: Oop) -> bool {
    oop.klass::<COMPRESSED>().id == KlassID::ObjArray
}

pub fn is_val_array<const COMPRESSED: bool>(oop: Oop) -> bool {
    oop.klass::<COMPRESSED>().id == KlassID::TypeArray
}

pub fn obj_array_data<const COMPRESSED: bool>(oop: Oop) -> crate::OpenJDKEdgeRange<COMPRESSED> {
    unsafe {
        let array = oop.as_array_oop();
        array.slice::<COMPRESSED>(BasicType::T_OBJECT)
    }
}

thread_local! {
    static CLOSURE: UnsafeCell<*mut u8> = UnsafeCell::new(std::ptr::null_mut());
}

pub unsafe extern "C" fn scan_object_fn<E: Edge, V: EdgeVisitor<E>>(edge: Address) {
    let ptr: *mut u8 = CLOSURE.with(|x| *x.get());
    let closure = &mut *(ptr as *mut V);
    closure.visit_edge(E::from_address(edge));
}

pub fn scan_object<E: Edge + From<Address>, V: EdgeVisitor<E>, const COMPRESSED: bool>(
    object: ObjectReference,
    closure: &mut V,
    _tls: VMWorkerThread,
) {
    unsafe { oop_iterate::<E, V, COMPRESSED>(mem::transmute(object), closure, None) }
}

pub fn scan_object_with_klass<
    E: Edge + From<Address>,
    V: EdgeVisitor<E>,
    const COMPRESSED: bool,
>(
    object: ObjectReference,
    closure: &mut V,
    _tls: VMWorkerThread,
    klass: Address,
) {
    unsafe { oop_iterate::<E, V, COMPRESSED>(mem::transmute(object), closure, Some(klass)) }
}
