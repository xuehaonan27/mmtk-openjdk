
#include "precompiled.hpp"
#include "mmtk.h"
#include "mmtkMutator.hpp"
#include "mmtkHeap.hpp"

size_t MMTkMutatorContext::max_non_los_default_alloc_bytes = 0;

MMTkMutatorContext MMTkMutatorContext::bind(::Thread* current) {
  if (IMMIX_ALLOCATOR_SIZE != sizeof(ImmixAllocator)) {
    printf("ERROR: Unmatched immix allocator size: rs=%zu cpp=%zu\n", IMMIX_ALLOCATOR_SIZE, sizeof(ImmixAllocator));
    guarantee(false, "ERROR");
  }
  return *((MMTkMutatorContext*) ::bind_mutator((void*) current));
}

bool MMTkMutatorContext::is_ready_to_bind() {
  return ::openjdk_is_gc_initialized();
}

void MMTkMutatorContext::flush() {
  ::flush_mutator((MMTk_Mutator) this);
}
