#include "c1/c1_CodeStubs.hpp"
#include "gc/shared/c1/barrierSetC1.hpp"
#include "mmtkBarrierSetAssembler_x86.hpp"
#include "mmtkBarrierSetC1.hpp"

void MMTkBarrierSetC1::generate_c1_runtime_stubs(BufferBlob* buffer_blob) {
  class MMTkBarrierCodeGenClosure : public StubAssemblerCodeGenClosure {
    bool _do_code_patch;
    virtual OopMapSet* generate_code(StubAssembler* sasm) override {
      MMTkBarrierSetAssembler* bs = (MMTkBarrierSetAssembler*) BarrierSet::barrier_set()->barrier_set_assembler();
      bs->generate_c1_write_barrier_runtime_stub(sasm, _do_code_patch);
      return NULL;
    }
  public:
    MMTkBarrierCodeGenClosure(bool do_code_patch): _do_code_patch(do_code_patch) {}
  };
  MMTkBarrierCodeGenClosure write_code_gen_cl(false);
  _write_barrier_c1_runtime_code_blob = Runtime1::generate_blob(buffer_blob, -1, "write_code_gen_cl", false, &write_code_gen_cl);
  MMTkBarrierCodeGenClosure write_code_gen_cl_patch_fix(true);
  _write_barrier_c1_runtime_code_blob_with_patch_fix = Runtime1::generate_blob(buffer_blob, -1, "write_code_gen_cl_patch_fix", false, &write_code_gen_cl_patch_fix);
  
  class MMTkRefLoadBarrierCodeGenClosure : public StubAssemblerCodeGenClosure {
    virtual OopMapSet* generate_code(StubAssembler* sasm) override {
      MMTkBarrierSetAssembler* bs = (MMTkBarrierSetAssembler*) BarrierSet::barrier_set()->barrier_set_assembler();
      bs->generate_c1_ref_load_barrier_runtime_stub(sasm);
      return NULL;
    }
  };
  MMTkRefLoadBarrierCodeGenClosure load_code_gen_cl;
  _ref_load_barrier_c1_runtime_code_blob = Runtime1::generate_blob(buffer_blob, -1, "load_code_gen_cl", false, &load_code_gen_cl);
}

void MMTkC1BarrierStub::emit_code(LIR_Assembler* ce) {
  MMTkBarrierSetAssembler* bs = (MMTkBarrierSetAssembler*) BarrierSet::barrier_set()->barrier_set_assembler();
  bs->generate_c1_write_barrier_stub_call(ce, this);
}

void MMTkC1ReferenceLoadBarrierStub::emit_code(LIR_Assembler* ce) {
  MMTkBarrierSetAssembler* bs = (MMTkBarrierSetAssembler*) BarrierSet::barrier_set()->barrier_set_assembler();
  bs->generate_c1_ref_load_barrier_stub_call(ce, this);
}
