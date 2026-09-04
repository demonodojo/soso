//! Tests del planificador por operación.

use soso_llm_core::{Dest, OpDesc, OpSched, CostModel};
use sosomodel::layout::DTYPE_F32;

#[test]
fn pequeno_va_a_cpu() {
    let mut sched = OpSched::default();
    let op = OpDesc::matvec(384, 384, true, DTYPE_F32);
    assert_eq!(sched.elegir(&op, true), Dest::Cpu);
}

#[test]
fn grande_va_a_gpu() {
    let mut sched = OpSched::new(CostModel {
        gpu_fixed_ns: 100_000.0,
        gpu_ns_per_mac: 0.01,
        gpu_ns_per_byte: 0.5,
        cpu_ns_per_mac: 1.0,
        gpu_upload_ns: 0.0,
    });
    let op = OpDesc::matmul(4096, 4096, 1, true, DTYPE_F32);
    assert_eq!(sched.elegir(&op, true), Dest::Gpu);
}

#[test]
fn batch_grande_va_a_gpu() {
    let mut sched = OpSched::new(CostModel {
        gpu_fixed_ns: 200_000.0,
        gpu_ns_per_mac: 0.02,
        gpu_ns_per_byte: 0.5,
        cpu_ns_per_mac: 0.5,
        gpu_upload_ns: 0.0,
    });
    let op = OpDesc::matmul(384, 384, 1500, true, DTYPE_F32);
    assert_eq!(sched.elegir(&op, true), Dest::Gpu);
}

#[test]
fn registrar_mueve_decision() {
    let mut sched = OpSched::default();
    let op = OpDesc::matvec(384, 384, true, DTYPE_F32);
    for _ in 0..20 {
        sched.registrar(&op, Dest::Gpu, 50_000);
    }
    assert!(sched.ewma_gpu(0) < sched.estimate_cpu_ns(&op));
}
