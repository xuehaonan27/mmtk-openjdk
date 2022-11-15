
#ifndef MMTK_OPENJDK_MMTK_MUTATOR_INLINE_HPP
#define MMTK_OPENJDK_MMTK_MUTATOR_INLINE_HPP

#include "mmtk.h"
#include "utilities/globalDefinitions.hpp"
#include "mmtkMutator.hpp"
#include "mmtkHeap.hpp"


inline HeapWord* MMTkMutatorContext::alloc(size_t bytes, Allocator allocator) {
  // All allocations with size larger than max non-los bytes will get to this slowpath here.
  // We will use LOS for those.
  assert(MMTkMutatorContext::max_non_los_default_alloc_bytes != 0, "max_non_los_default_alloc_bytes hasn't been initialized");
  if (bytes >= MMTkMutatorContext::max_non_los_default_alloc_bytes) {
    allocator = AllocatorLos;
  } else {
    AllocatorSelector selector = MMTkHeap::heap()->default_allocator_selector;
    if (selector.tag == TAG_IMMIX) {
      auto& allocator = allocators.immix[selector.index];
      auto cursor = uintptr_t(allocator.cursor);
      auto limit = uintptr_t(allocator.limit);
      if (cursor + bytes <= limit) {
        allocator.cursor = (void*) (cursor + bytes);
        return (HeapWord*) cursor;
      } else if (bytes > 256) {
        auto large_cursor = uintptr_t(allocator.large_cursor);
        auto large_limit = uintptr_t(allocator.large_limit);
        if (large_cursor + bytes <= large_limit) {
          allocator.large_cursor = (void*) (large_cursor + bytes);
          return (HeapWord*) large_cursor;
        }
      }
    }
  }

  // FIXME: Proper use of slow-path api
  HeapWord* o = (HeapWord*) ::alloc((MMTk_Mutator) this, bytes, HeapWordSize, 0, allocator);
  // Post allocation hooks. Note that we can get a nullptr from mmtk core in the case of OOM.
  // Hence, only call post allocation hooks if we have a proper object.
  if (o != nullptr && allocator != AllocatorDefault) {
    ::post_alloc((MMTk_Mutator) this, o, bytes, allocator);
  }
  return o;
}

#endif // MMTK_OPENJDK_MMTK_MUTATOR_INLINE_HPP
