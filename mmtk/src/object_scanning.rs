use super::abi::*;
use super::UPCALLS;
use mmtk::util::constants::*;
use mmtk::util::opaque_pointer::*;
use mmtk::util::{Address, ObjectReference};
use mmtk::vm::EdgeVisitor;
use std::ffi::CStr;
use std::{mem, slice};

trait OopIterate: Sized {
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor);
}

impl OopIterate for OopMapBlock {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
        let start = oop.get_field_address(self.offset);
        for i in 0..self.count as usize {
            let edge = start + (i << 2);
            closure.visit_edge(edge);
        }
    }
}

impl OopIterate for InstanceKlass {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
        let oop_maps = self.nonstatic_oop_maps();
        for map in oop_maps {
            map.oop_iterate(oop, closure)
        }
    }
}

impl OopIterate for InstanceMirrorKlass {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
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
        let start: *const NarrowOop = Self::start_of_static_fields(oop).to_ptr::<NarrowOop>();
        let len = Self::static_oop_field_count(oop);
        let slice = unsafe { slice::from_raw_parts(start, len as _) };
        for narrow_oop in slice {
            closure.visit_edge(narrow_oop.slot());
        }
    }
}

impl OopIterate for InstanceClassLoaderKlass {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
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
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
        let array = unsafe { oop.as_array_oop() };
        for narrow_oop in unsafe { array.data::<NarrowOop>(BasicType::T_OBJECT) } {
            closure.visit_edge(narrow_oop.slot());
        }
    }
}

impl OopIterate for TypeArrayKlass {
    #[inline]
    fn oop_iterate(&self, _oop: Oop, _closure: &mut impl EdgeVisitor) {
        // Performance tweak: We skip processing the klass pointer since all
        // TypeArrayKlasses are guaranteed processed via the null class loader.
    }
}

impl OopIterate for InstanceRefKlass {
    #[inline]
    fn oop_iterate(&self, oop: Oop, closure: &mut impl EdgeVisitor) {
        use crate::abi::*;
        use crate::api::{add_phantom_candidate, add_soft_candidate, add_weak_candidate};
        self.instance_klass.oop_iterate(oop, closure);

        if Self::should_scan_weak_refs() {
            unreachable!();
            let reference = ObjectReference::from(oop);
            match self.instance_klass.reference_type {
                ReferenceType::None => {
                    panic!("oop_iterate on InstanceRefKlass with reference_type as None")
                }
                ReferenceType::Weak => add_weak_candidate(reference),
                ReferenceType::Soft => add_soft_candidate(reference),
                ReferenceType::Phantom => add_phantom_candidate(reference),
                // Process these two types normally (as if they are strong refs)
                // We will handle final reference later
                ReferenceType::Final | ReferenceType::Other => {
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
    fn should_scan_weak_refs() -> bool {
        use SINGLETON;
        !*SINGLETON.get_options().no_reference_types
    }
    #[inline]
    fn process_ref_as_strong(oop: Oop, closure: &mut impl EdgeVisitor) {
        let referent_addr = Self::referent_address(oop);
        closure.visit_edge(referent_addr);
        let discovered_addr = Self::discovered_address(oop);
        closure.visit_edge(discovered_addr);
    }
}

#[allow(unused)]
fn oop_iterate_slow(oop: Oop, process_edge: extern "C" fn(Address), tls: OpaquePointer) {
    unsafe {
        ((*UPCALLS).scan_object)(process_edge as _, mem::transmute(oop), tls);
    }
}

static mut FIELDS: Vec<Address> = Vec::new();

#[inline]
fn oop_iterate(oop: Oop, closure: &mut impl EdgeVisitor) {
    unsafe {
        FIELDS = Vec::new();
    }
    extern "C" fn report_edge(a: Address) {
        unsafe { FIELDS.push(a) }
    }
    oop_iterate_slow(oop, report_edge, OpaquePointer::UNINITIALIZED);
    unsafe {
        for e in &FIELDS {
            closure.visit_edge(*e)
        }
    }
    // let klass = oop.klass();
    // let klass_id = oop.klass().id;
    // debug_assert!(
    //     klass_id as i32 >= 0 && (klass_id as i32) < 6,
    //     "Invalid klass-id: {:x} for oop: {:x}",
    //     klass_id as i32,
    //     unsafe { mem::transmute::<Oop, ObjectReference>(oop) }
    // );
    // unsafe {
    //     ((*UPCALLS).dump_object)(mem::transmute(oop));
    // }
    // // println!("oop {:?}", unsafe {
    // //     let c_string = ((*UPCALLS).dump_object)(mem::transmute(oop));
    // //     let c_str: &CStr = unsafe { CStr::from_ptr(c_string) };
    // //     c_str
    // // });
    // match klass_id {
    //     KlassID::Instance => {
    //         let instance_klass = unsafe { klass.cast::<InstanceKlass>() };
    //         instance_klass.oop_iterate(oop, closure);
    //     }
    //     KlassID::InstanceClassLoader => {
    //         let instance_klass = unsafe { klass.cast::<InstanceClassLoaderKlass>() };
    //         instance_klass.oop_iterate(oop, closure);
    //     }
    //     KlassID::InstanceMirror => {
    //         let instance_klass = unsafe { klass.cast::<InstanceMirrorKlass>() };
    //         instance_klass.oop_iterate(oop, closure);
    //     }
    //     KlassID::ObjArray => {
    //         let array_klass = unsafe { klass.cast::<ObjArrayKlass>() };
    //         array_klass.oop_iterate(oop, closure);
    //     }
    //     KlassID::TypeArray => {
    //         let array_klass = unsafe { klass.cast::<TypeArrayKlass>() };
    //         array_klass.oop_iterate(oop, closure);
    //     }
    //     KlassID::InstanceRef => {
    //         let instance_klass = unsafe { klass.cast::<InstanceRefKlass>() };
    //         instance_klass.oop_iterate(oop, closure);
    //     } // _ => oop_iterate_slow(oop, closure, tls),
    // }
}

#[inline]
pub fn scan_object<T: EdgeVisitor>(object: ObjectReference, closure: &mut T, _tls: VMWorkerThread) {
    // println!(
    //     "Scan {:?} (klass={:?} id={:?})",
    //     object,
    //     unsafe { (object.to_address() + 8usize).load::<Address>() },
    //     unsafe { mem::transmute::<_, Oop>(object).klass().id }
    // );
    // println!("*****scan_object(0x{:x}) -> \n 0x{:x}, 0x{:x} \n",
    //     object,
    //     unsafe { *(object.value() as *const usize) },
    //     unsafe { *((object.value() + 8) as *const usize) }
    // );
    let closure = unsafe { &mut *(closure as *mut T) };
    unsafe { oop_iterate(mem::transmute(object), closure) }
}
