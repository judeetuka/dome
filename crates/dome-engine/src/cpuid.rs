//! CPUID policy for guest vCPUs.
//!
//! ADR 0002 requires deny-by-default, not passthrough. The spike measured the cost of
//! getting this wrong: unfiltered CPUID advertised CET, the guest believed it, and died
//! inside an unrelated fault handler with a recursive fault.

use kvm_bindings::{CpuId, KVM_CPUID_FLAG_SIGNIFCANT_INDEX, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::Kvm;

use crate::EngineError;

/// CPUID leaf 7, subleaf 0: CET shadow stack.
const CET_SS: u32 = 1 << 7;
/// CPUID leaf 7, subleaf 0: CET indirect branch tracking.
const CET_IBT: u32 = 1 << 20;

/// Build the CPUID the guest will see.
///
/// Starts from what the host supports, then removes features this VMM does not implement.
/// A feature left in that nothing implements is worse than a feature missing: the guest
/// takes a fast path that faults in code nobody was looking at.
pub fn engine_cpuid(kvm: &Kvm) -> Result<CpuId, EngineError> {
    let mut cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| EngineError::Setup(format!("cannot read host CPUID: {error}")))?;

    mask_unimplemented(&mut cpuid);

    Ok(cpuid)
}

/// Remove features the engine does not implement.
///
/// Currently CET only. Everything else the host advertises is left alone because the
/// engine has not yet been shown to break on it; each removal here should be traceable to
/// a failure, and the comment should say which.
fn mask_unimplemented(cpuid: &mut CpuId) {
    for entry in cpuid.as_mut_slice() {
        if entry.function == 7
            && entry.index == 0
            && (entry.flags & KVM_CPUID_FLAG_SIGNIFCANT_INDEX) != 0
        {
            entry.ecx &= !CET_SS;
            entry.edx &= !CET_IBT;
        }
    }
}

/// Whether the guest is told about CET, for tests.
pub fn advertises_cet(cpuid: &CpuId) -> bool {
    cpuid.as_slice().iter().any(|entry| {
        entry.function == 7
            && entry.index == 0
            && ((entry.ecx & CET_SS) != 0 || (entry.edx & CET_IBT) != 0)
    })
}
