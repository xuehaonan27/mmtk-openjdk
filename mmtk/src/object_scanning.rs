use crate::reference_glue::DISCOVERED_LISTS;
use crate::SINGLETON;

use super::abi::*;
use super::{OpenJDKEdge, UPCALLS};
use mmtk::util::constants::*;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::EdgeVisitor;
use std::{mem, slice};

trait OopIterate: Sized {
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>);
    fn oop_iterate_with_discovery(
        &self,
        _oop: Oop,
        _closure: &mut impl EdgeVisitor<OpenJDKEdge>,
        _discover: bool,
    ) {
        unimplemented!()
    }
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
        // if (Devirtualizer::do_metadata(closure)) {
        //     Klass* klass = java_lang_Class::as_Klass(obj);
        //     // We'll get NULL for primitive mirrors.
        //     if (klass != NULL) {
        //       if (klass->is_instance_klass() && InstanceKlass::cast(klass)->is_anonymous()) {
        //         // An anonymous class doesn't have its own class loader, so when handling
        //         // the java mirror for an anonymous class we need to make sure its class
        //         // loader data is claimed, this is done by calling do_cld explicitly.
        //         // For non-anonymous classes the call to do_cld is made when the class
        //         // loader itself is handled.
        //         Devirtualizer::do_cld(closure, klass->class_loader_data());
        //       } else {
        //         Devirtualizer::do_klass(closure, klass);
        //       }
        //     } else {
        //       // We would like to assert here (as below) that if klass has been NULL, then
        //       // this has been a mirror for a primitive type that we do not need to follow
        //       // as they are always strong roots.
        //       // However, we might get across a klass that just changed during CMS concurrent
        //       // marking if allocation occurred in the old generation.
        //       // This is benign here, as we keep alive all CLDs that were loaded during the
        //       // CMS concurrent phase in the class loading, i.e. they will be iterated over
        //       // and kept alive during remark.
        //       // assert(java_lang_Class::is_primitive(obj), "Sanity check");
        //     }
        // }

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
        // if (Devirtualizer::do_metadata(closure)) {
        //     ClassLoaderData* cld = java_lang_ClassLoader::loader_data(obj);
        //     // cld can be null if we have a non-registered class loader.
        //     if (cld != NULL) {
        //         Devirtualizer::do_cld(closure, cld);
        //     }
        // }
    }
}

impl OopIterate for ObjArrayKlass {
    #[inline(always)]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>) {
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
        unreachable!()
    }

    fn oop_iterate_with_discovery(
        &self,
        oop: Oop,
        closure: &mut impl EdgeVisitor<OpenJDKEdge>,
        disable_discovery: bool,
    ) {
        use crate::abi::*;
        self.instance_klass.oop_iterate(oop, closure);

        if Self::should_discover_refs(self.instance_klass.reference_type, disable_discovery) {
            match self.instance_klass.reference_type {
                ReferenceType::None => {
                    panic!("oop_iterate on InstanceRefKlass with reference_type as None")
                }
                rt @ (ReferenceType::Weak | ReferenceType::Phantom | ReferenceType::Soft) => {
                    if !Self::discover_reference(oop, rt) {
                        Self::process_ref_as_strong(oop, closure)
                    }
                }
                // Process these two types normally (as if they are strong refs)
                // We will handle final reference later
                // ReferenceType::Weak
                // | ReferenceType::Phantom
                ReferenceType::Other | ReferenceType::Final => {
                    Self::process_ref_as_strong(oop, closure)
                }
            }
        } else {
            Self::process_ref_as_strong(oop, closure);
        }
    }
}

impl InstanceRefKlass {
    #[inline]
    fn should_discover_refs(rt: ReferenceType, disable_discovery: bool) -> bool {
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
        use crate::api::{add_phantom_candidate, add_soft_candidate, add_weak_candidate};
        // Do not discover new refs during reference processing.
        if crate::VM_REF_PROCESSOR {
            if !DISCOVERED_LISTS.allow_discover() {
                return false;
            }
        } else {
            if !crate::SINGLETON.reference_processors.allow_new_candidate() {
                return false;
            }
        }
        // Do not discover if the referent is live.
        let referent: ObjectReference = unsafe { InstanceRefKlass::referent_address(oop).load() };
        // Skip live or null referents
        if referent.is_reachable() || referent.is_null() {
            return false;
        }
        // Skip young referents
        if mmtk::util::rc::count(referent) == 0 {
            return false;
        }
        // TODO: Do not discover if the referent is a nursery object.

        // Add to reference list
        let reference: ObjectReference = oop.into();
        if crate::VM_REF_PROCESSOR {
            // crate::reference_glue::set_referent(reference, ObjectReference::NULL);
            DISCOVERED_LISTS.get(rt).add(reference);
        } else {
            match rt {
                ReferenceType::Weak => add_weak_candidate(reference),
                ReferenceType::Soft => add_soft_candidate(reference),
                ReferenceType::Phantom => add_phantom_candidate(reference),
                _ => unreachable!(),
            }
        }
        true
    }
}

#[allow(unused)]
fn oop_iterate_slow(oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>, tls: OpaquePointer) {
    unsafe {
        ((*UPCALLS).scan_object)(closure as *mut _ as _, mem::transmute(oop), tls);
    }
}

#[inline(always)]
fn oop_iterate(oop: Oop, closure: &mut impl EdgeVisitor<OpenJDKEdge>, disable_discovery: bool) {
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
            instance_klass.oop_iterate_with_discovery(oop, closure, disable_discovery);
        } // _ => oop_iterate_slow(oop, closure, tls),
        _ => {}
    }
}

#[inline(always)]
pub fn is_obj_array(oop: Oop) -> bool {
    let klass_id = oop.klass.id;
    klass_id == KlassID::ObjArray
}

#[inline(always)]
pub fn obj_array_data(oop: Oop) -> &'static [ObjectReference] {
    unsafe {
        let array = oop.as_array_oop();
        array.data::<ObjectReference>(BasicType::T_OBJECT)
    }
}

#[inline]
pub fn scan_object(
    object: ObjectReference,
    closure: &mut impl EdgeVisitor<OpenJDKEdge>,
    _tls: VMWorkerThread,
    discover_references: bool,
) {
    // println!("*****scan_object(0x{:x}) -> \n 0x{:x}, 0x{:x} \n",
    //     object,
    //     unsafe { *(object.value() as *const usize) },
    //     unsafe { *((object.value() + 8) as *const usize) }
    // );
    unsafe { oop_iterate(mem::transmute(object), closure, discover_references) }
}
