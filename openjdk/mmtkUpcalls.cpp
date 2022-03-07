/*
 * Copyright (c) 2017, Red Hat, Inc. and/or its affiliates.
 * DO NOT ALTER OR REMOVE COPYRIGHT NOTICES OR THIS FILE HEADER.
 *
 * This code is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 2 only, as
 * published by the Free Software Foundation.
 *
 * This code is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
 * FITNESS FOR A PARTICULAR PURPOSE.  See the GNU General Public License
 * version 2 for more details (a copy is included in the LICENSE file that
 * accompanied this code).
 *
 * You should have received a copy of the GNU General Public License version
 * 2 along with this work; if not, write to the Free Software Foundation,
 * Inc., 51 Franklin St, Fifth Floor, Boston, MA 02110-1301 USA.
 *
 * Please contact Sun 500 Oracle Parkway, Redwood Shores, CA 94065 USA
 * or visit www.oracle.com if you need additional information or have any
 * questions.
 *
 */

#include "precompiled.hpp"
#include "classfile/stringTable.hpp"
#include "code/nmethod.hpp"
#include "memory/iterator.inline.hpp"
#include "memory/resourceArea.hpp"
#include "mmtkCollectorThread.hpp"
#include "mmtkContextThread.hpp"
#include "mmtkHeap.hpp"
#include "mmtkRootsClosure.hpp"
#include "mmtkUpcalls.hpp"
#include "mmtkVMCompanionThread.hpp"
#include "runtime/mutexLocker.hpp"
#include "runtime/os.hpp"
#include "runtime/safepoint.hpp"
#include "runtime/thread.hpp"
#include "runtime/threadSMR.hpp"
#include "runtime/vmThread.hpp"
#include "gc/shared/weakProcessor.hpp"
#include "prims/resolvedMethodTable.hpp"
#include "jfr/jfr.hpp"
#include "gc/shared/oopStorage.inline.hpp"

static size_t mmtk_start_the_world_count = 0;

class MMTkSSIsAliveClosure : public BoolObjectClosure {
  static constexpr size_t SS0_START = 0x20000000000ULL;
  static constexpr size_t SS1_START = 0x40000000000ULL;
  static constexpr size_t LOS_START = 0x80000000000ULL;
  void* from_start = NULL;
  void* from_limit = NULL;
  void* to_start = NULL;
  void* to_limit = NULL;
public:
  MMTkSSIsAliveClosure() {
    from_start = (void*) (high ? SS0_START : SS1_START);
    from_limit = (void*) (((size_t) from_start) + 0x20000000000ULL);
    to_start = (void*) (high ? SS1_START : SS0_START);
    to_limit = (void*) (((size_t) to_start) + 0x20000000000ULL);
  }
  inline virtual bool do_object_b(oop p) {
    if (p == NULL) return false;
#if INLINE_IS_ALIVE
    auto x = (void*) p;
    if (x >= to_start && x < to_limit) {
      return true;
    }
    if (x >= from_start && x < from_limit) {
      auto status = *((size_t*) (void*) p);
      return (status & (3ull << 56)) != 0;
    }
    if (x >= (void*) LOS_START && x < ((void*) (LOS_START + 0x20000000000ULL))) {
      return mmtk_is_live((void*) p) != 0;;
    }
    return false;
#else
    return mmtk_is_live((void*) p) != 0;
#endif
  }
};

class MMTkForwardClosure : public BasicOopIterateClosure {
 public:
  virtual void do_oop(oop* slot) {
    // *slot = (oop) mmtk_get_forwarded_ref((void*) *slot);
    auto o = *slot;
    if (o == NULL) return;
    auto status = *((size_t*) (void*) o);
    if ((status & (3ull << 56)) != 0) {
      auto ptr = (oop) (void*) (status << 8 >> 8);
      *slot = ptr;
    }
  }
  virtual void do_oop(narrowOop* o) {}
  virtual ReferenceIterationMode reference_iteration_mode() { return DO_FIELDS; }
};

static void mmtk_stop_all_mutators(void *tls, void (*create_stack_scan_work)(void* mutator)) {
  MMTkHeap::_create_stack_scan_work = create_stack_scan_work;

  // ClassLoaderDataGraph::clear_claimed_marks();
  // CodeCache::gc_prologue();
#if COMPILER2_OR_JVMCI
  DerivedPointerTable::clear();
#endif

  log_debug(gc)("Requesting the VM to suspend all mutators...");
  MMTkHeap::heap()->companion_thread()->request(MMTkVMCompanionThread::_threads_suspended, true);
  log_debug(gc)("Mutators stopped. Now enumerate threads for scanning...");
  mmtk_report_gc_start();

  nmethod::oops_do_marking_prologue();
  {
    JavaThreadIteratorWithHandle jtiwh;
    while (JavaThread *cur = jtiwh.next()) {
      MMTkHeap::heap()->report_java_thread_yield(cur);
    }
  }
  log_debug(gc)("Finished enumerating threads.");
}

static void mmtk_resume_mutators(void *tls) {
  {
    HandleMark hm;
    MMTkSSIsAliveClosure is_alive;
    MMTkForwardClosure forward;
    WeakProcessor::weak_oops_do(&is_alive, &forward);
  }
  // ClassLoaderDataGraph::purge();
  // CodeCache::gc_epilogue();
  // JvmtiExport::gc_epilogue();
  nmethod::oops_do_marking_epilogue();
  // ClassLoaderDataGraph::purge();
  // BiasedLocking::restore_marks();
  // CodeCache::gc_epilogue();
  // JvmtiExport::gc_epilogue();
#if COMPILER2_OR_JVMCI
  DerivedPointerTable::update_pointers();
#endif

  MMTkHeap::_create_stack_scan_work = NULL;

  log_debug(gc)("Requesting the VM to resume all mutators...");
  MMTkHeap::heap()->companion_thread()->request(MMTkVMCompanionThread::_threads_resumed, true);
  log_debug(gc)("Mutators resumed. Now notify any mutators waiting for GC to finish...");

  {
    MutexLockerEx locker(MMTkHeap::heap()->gc_lock(), true);
    mmtk_start_the_world_count++;
    MMTkHeap::heap()->gc_lock()->notify_all();
  }
  log_debug(gc)("Mutators notified.");
}

static void mmtk_spawn_collector_thread(void* tls, void* ctx) {
  if (ctx == NULL) {
    MMTkContextThread* t = new MMTkContextThread();
    if (!os::create_thread(t, os::pgc_thread)) {
      printf("Failed to create thread");
      guarantee(false, "panic");
    }
    os::start_thread(t);
  } else {
    MMTkHeap::heap()->new_collector_thread();
    MMTkCollectorThread* t = new MMTkCollectorThread(ctx);
    if (!os::create_thread(t, os::pgc_thread)) {
      printf("Failed to create thread");
      guarantee(false, "panic");
    }
    os::start_thread(t);
  }
}

static void mmtk_block_for_gc() {
  MMTkHeap::heap()->_last_gc_time = os::javaTimeNanos() / NANOSECS_PER_MILLISEC;
  log_debug(gc)("Thread (id=%d) will block waiting for GC to finish.", Thread::current()->osthread()->thread_id());
  {
    size_t my_count = mmtk_start_the_world_count;
    size_t next_count = my_count + 1;
    MutexLocker locker(MMTkHeap::heap()->gc_lock());

    while (mmtk_start_the_world_count < next_count) {
      MMTkHeap::heap()->gc_lock()->wait();
    }
  }
  log_debug(gc)("Thread (id=%d) resumed after GC finished.", Thread::current()->osthread()->thread_id());
}

static void* mmtk_get_mmtk_mutator(void* tls) {
  return (void*) &((Thread*) tls)->third_party_heap_mutator;
}

static bool mmtk_is_mutator(void* tls) {
  if (tls == NULL) return false;
  return ((Thread*) tls)->third_party_heap_collector == NULL;
}

template <class T>
struct MaybeUninit {
  MaybeUninit() {}
  T* operator->() {
    return (T*) &_data;
  }
  T& operator*() {
    return *((T*) &_data);
  }
private:
  char _data[sizeof(T)];
};

static MaybeUninit<JavaThreadIteratorWithHandle> jtiwh;
static bool mutator_iteration_start = true;

static void* mmtk_get_next_mutator() {
  if (mutator_iteration_start) {
    *jtiwh = JavaThreadIteratorWithHandle();
    mutator_iteration_start = false;
  }
  JavaThread *thr = jtiwh->next();
  if (thr == NULL) {
    mutator_iteration_start = true;
    return NULL;
  }
  return (void*) &thr->third_party_heap_mutator;
}

static void mmtk_reset_mutator_iterator() {
  mutator_iteration_start = true;
}


static void mmtk_compute_global_roots(void* trace, void* tls) {
  MMTkRootsClosure cl(trace);
  MMTkHeap::heap()->scan_global_roots(cl);
}

static void mmtk_compute_static_roots(void* trace, void* tls) {
  MMTkRootsClosure cl(trace);
  MMTkHeap::heap()->scan_static_roots(cl);
}

static void mmtk_compute_thread_roots(void* trace, void* tls) {
  MMTkRootsClosure cl(trace);
  MMTkHeap::heap()->scan_thread_roots(cl);
}

static void mmtk_scan_thread_roots(ProcessEdgesFn process_edges) {
  MMTkRootsClosure2 cl(process_edges);
  MMTkHeap::heap()->scan_thread_roots(cl);
}

static void mmtk_scan_thread_root(ProcessEdgesFn process_edges, void* tls) {
  ResourceMark rm;
  JavaThread* thread = (JavaThread*) tls;
  MMTkRootsClosure2 cl(process_edges);
  MarkingCodeBlobClosure cb_cl(&cl, false);
  thread->oops_do(&cl, &cb_cl);
}

static void mmtk_scan_object(void* trace, void* object, void* tls) {
  MMTkScanObjectClosure cl(trace);
  ((oop) object)->oop_iterate(&cl);
}

static void mmtk_dump_object(void* object) {
  oop o = (oop) object;

  // o->print();
  o->print_value();
  printf("\n");

  // o->print_address();
}

static size_t mmtk_get_object_size(void* object) {
  oop o = (oop) object;
  auto klass = o->klass();
  return klass->oop_size(o) << LogHeapWordSize;
}

static int mmtk_enter_vm() {
  assert(Thread::current()->is_Java_thread(), "Only Java thread can enter vm");

  JavaThread* current = ((JavaThread*) Thread::current());
  JavaThreadState state = current->thread_state();
  current->set_thread_state(_thread_in_vm);
  return (int)state;
}

static void mmtk_leave_vm(int st) {
  assert(Thread::current()->is_Java_thread(), "Only Java thread can leave vm");

  JavaThread* current = ((JavaThread*) Thread::current());
  assert(current->thread_state() == _thread_in_vm, "Cannot leave vm when the current thread is not in _thread_in_vm");
  current->set_thread_state((JavaThreadState)st);
}

static int offset_of_static_fields() {
  return InstanceMirrorKlass::offset_of_static_fields();
}

static int static_oop_field_count_offset() {
  return java_lang_Class::static_oop_field_count_offset();
}

static size_t compute_klass_mem_layout_checksum() {
  return sizeof(Klass)
    ^ sizeof(InstanceKlass)
    ^ sizeof(InstanceRefKlass)
    ^ sizeof(InstanceMirrorKlass)
    ^ sizeof(InstanceClassLoaderKlass)
    ^ sizeof(TypeArrayKlass)
    ^ sizeof(ObjArrayKlass);
}

static int referent_offset() {
  return java_lang_ref_Reference::referent_offset;
}

static int discovered_offset() {
  return java_lang_ref_Reference::discovered_offset;
}

static char* dump_object_string(void* object) {
  oop o = (oop) object;
  return o->print_value_string();
}

static void mmtk_schedule_finalizer() {
  MMTkHeap::heap()->schedule_finalizer();
}

static void mmtk_scan_universe_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_universe_roots(cl); }
static void mmtk_scan_jni_handle_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_jni_handle_roots(cl); }
static void mmtk_scan_object_synchronizer_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_object_synchronizer_roots(cl); }
static void mmtk_scan_management_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_management_roots(cl); }
static void mmtk_scan_jvmti_export_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_jvmti_export_roots(cl); }
static void mmtk_scan_aot_loader_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_aot_loader_roots(cl); }
static void mmtk_scan_system_dictionary_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_system_dictionary_roots(cl); }
static void mmtk_scan_code_cache_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_code_cache_roots(cl); }
static void mmtk_scan_string_table_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_string_table_roots(cl); }
static void mmtk_scan_class_loader_data_graph_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_class_loader_data_graph_roots(cl); }
static void mmtk_scan_weak_processor_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_weak_processor_roots(cl); }
static void mmtk_scan_vm_thread_roots(ProcessEdgesFn process_edges) { MMTkRootsClosure2 cl(process_edges); MMTkHeap::heap()->scan_vm_thread_roots(cl); }

static size_t mmtk_number_of_mutators() {
  return Threads::number_of_threads();
}

static void mmtk_prepare_for_roots_re_scanning() {
#if COMPILER2_OR_JVMCI
  DerivedPointerTable::update_pointers();
  DerivedPointerTable::clear();
#endif
}

static int32_t mmtk_object_alignment() {
  ShouldNotReachHere();
  return 0;
}

// class MMTkIsAliveClosure : public BoolObjectClosure {
// public:
//   virtual bool do_object_b(oop p) {
//     if (p == NULL) return false;
//     return mmtk_is_live((void*) p) != 0;
//   }
// };

// class MMTkForwardClosure : public BasicOopIterateClosure {
//  public:
//   virtual void do_oop(oop* slot) {
//     // *slot = (oop) mmtk_get_forwarded_ref((void*) *slot);
//     auto o = *slot;
//     if (o == NULL) return;
//     auto status = *((size_t*) (void*) o);
//     if ((status & (3ull << 56)) != 0) {
//       auto ptr = (oop) (void*) (status << 8 >> 8);
//       *slot = ptr;
//     }
//   }
//   virtual void do_oop(narrowOop* o) {}
//   virtual ReferenceIterationMode reference_iteration_mode() { return DO_FIELDS; }
// };

/// Clean up the weak-ref storage and update pointers.
static void mmtk_process_weak_ref(int id) {
//   HandleMark hm;

//   MMTkIsAliveClosure is_alive;
//   MMTkForwardClosure forward;

//   JNIHandles::weak_oops_do(&is_alive, &forward);
//   JvmtiExport::weak_oops_do(&is_alive, &forward);
//   SystemDictionary::vm_weak_oop_storage()->weak_oops_do(&is_alive, &forward);
//   JFR_ONLY(Jfr::weak_oops_do(&is_alive, &forward););
//   // if (id == 0) JNIHandles::weak_oops_do(&is_alive, &forward);
//   // else if (id == 1) JvmtiExport::weak_oops_do(&is_alive, &forward);
//   // else if (id == 2) SystemDictionary::vm_weak_oop_storage()->weak_oops_do(&is_alive, &forward);
//   // else {
//   //   JFR_ONLY(Jfr::weak_oops_do(&is_alive, &forward););
//   // }
// printf("mmtk_process_weak_ref end\n");
}

static void mmtk_process_nmethods() {
  // HandleMark hm;
  // nmethod::oops_do_marking_epilogue();
}

OpenJDK_Upcalls mmtk_upcalls = {
  mmtk_stop_all_mutators,
  mmtk_resume_mutators,
  mmtk_spawn_collector_thread,
  mmtk_block_for_gc,
  mmtk_get_next_mutator,
  mmtk_reset_mutator_iterator,
  mmtk_compute_static_roots,
  mmtk_compute_global_roots,
  mmtk_compute_thread_roots,
  mmtk_scan_object,
  mmtk_dump_object,
  mmtk_get_object_size,
  mmtk_get_mmtk_mutator,
  mmtk_is_mutator,
  mmtk_enter_vm,
  mmtk_leave_vm,
  compute_klass_mem_layout_checksum,
  offset_of_static_fields,
  static_oop_field_count_offset,
  referent_offset,
  discovered_offset,
  dump_object_string,
  mmtk_scan_thread_roots,
  mmtk_scan_thread_root,
  mmtk_scan_universe_roots,
  mmtk_scan_jni_handle_roots,
  mmtk_scan_object_synchronizer_roots,
  mmtk_scan_management_roots,
  mmtk_scan_jvmti_export_roots,
  mmtk_scan_aot_loader_roots,
  mmtk_scan_system_dictionary_roots,
  mmtk_scan_code_cache_roots,
  mmtk_scan_string_table_roots,
  mmtk_scan_class_loader_data_graph_roots,
  mmtk_scan_weak_processor_roots,
  mmtk_scan_vm_thread_roots,
  mmtk_number_of_mutators,
  mmtk_schedule_finalizer,
  mmtk_prepare_for_roots_re_scanning,
  mmtk_object_alignment,
  mmtk_process_weak_ref,
  mmtk_process_nmethods,
};
