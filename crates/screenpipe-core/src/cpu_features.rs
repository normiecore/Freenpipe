// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! CPU feature checks for native code that needs a particular instruction set.
//!
//! The stock bun build for Windows x64 executes AVX2 instructions and dies with
//! `STATUS_ILLEGAL_INSTRUCTION` on processors (and VMs) that lack them; the
//! agent runtime uses [`cpu_has_avx2`] to pick bun's baseline build there.
//!
//! The decision is a pure function of the CPUID and XCR0 register values
//! ([`avx2_usable`]) so it can be tested with synthetic values on any machine.
//! On architectures other than x86_64 the question does not arise and
//! [`cpu_has_avx2`] answers `None`.

/// CPUID leaf 1, ECX: the OS has enabled XSAVE/XGETBV (`OSXSAVE`).
const LEAF1_ECX_OSXSAVE: u32 = 1 << 27;
/// CPUID leaf 1, ECX: the processor implements AVX.
const LEAF1_ECX_AVX: u32 = 1 << 28;
/// CPUID leaf 7 sub-leaf 0, EBX: the processor implements AVX2.
const LEAF7_EBX_AVX2: u32 = 1 << 5;
/// XCR0 bits 1 and 2: the OS saves SSE (XMM) and AVX (upper YMM) state on a
/// context switch. Without both, using YMM registers corrupts other threads.
const XCR0_SSE_AND_AVX_STATE: u64 = 0b110;

/// Register values read from the processor that decide AVX2 support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuidSnapshot {
    /// Highest standard leaf (`EAX` of leaf 0).
    pub max_leaf: u32,
    /// `ECX` of leaf 1.
    pub leaf1_ecx: u32,
    /// `EBX` of leaf 7, sub-leaf 0. Only meaningful when `max_leaf >= 7`.
    pub leaf7_ebx: u32,
    /// `XCR0` as returned by `XGETBV(0)`. Only meaningful when `OSXSAVE` is set.
    pub xcr0: u64,
}

/// Whether AVX2 instructions can run, from raw register values.
///
/// All of these must hold: leaf 7 exists, the CPU reports AVX and AVX2, the OS
/// has enabled XSAVE, and the OS saves both XMM and YMM state.
pub fn avx2_usable(snapshot: CpuidSnapshot) -> bool {
    if snapshot.max_leaf < 7 {
        return false;
    }
    let ecx = snapshot.leaf1_ecx;
    if ecx & LEAF1_ECX_OSXSAVE == 0 || ecx & LEAF1_ECX_AVX == 0 {
        return false;
    }
    if snapshot.xcr0 & XCR0_SSE_AND_AVX_STATE != XCR0_SSE_AND_AVX_STATE {
        return false;
    }
    snapshot.leaf7_ebx & LEAF7_EBX_AVX2 != 0
}

/// Read the registers [`avx2_usable`] needs from the running processor.
#[cfg(target_arch = "x86_64")]
pub fn read_cpuid_snapshot() -> CpuidSnapshot {
    use std::arch::x86_64::{__cpuid, __cpuid_count};

    // CPUID exists on every x86_64 processor (the intrinsic is safe).
    let leaf0 = __cpuid(0);
    let max_leaf = leaf0.eax;
    if max_leaf < 1 {
        return CpuidSnapshot {
            max_leaf,
            ..Default::default()
        };
    }
    let leaf1 = __cpuid(1);
    let leaf7_ebx = if max_leaf >= 7 {
        __cpuid_count(7, 0).ebx
    } else {
        0
    };
    let xcr0 = if leaf1.ecx & LEAF1_ECX_OSXSAVE != 0 {
        // SAFETY: OSXSAVE set means the OS enabled XGETBV, so the instruction
        // does not fault.
        unsafe { xgetbv0() }
    } else {
        0
    };
    CpuidSnapshot {
        max_leaf,
        leaf1_ecx: leaf1.ecx,
        leaf7_ebx,
        xcr0,
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "xsave")]
unsafe fn xgetbv0() -> u64 {
    std::arch::x86_64::_xgetbv(0)
}

/// Whether the running processor can execute AVX2 code.
///
/// `Some(true|false)` on x86_64, read with CPUID; `None` elsewhere, where
/// AVX2 is not a meaningful question (callers treat it as "no workaround
/// needed").
pub fn cpu_has_avx2() -> Option<bool> {
    #[cfg(target_arch = "x86_64")]
    {
        Some(avx2_usable(read_cpuid_snapshot()))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Haswell-class machine with an AVX-aware OS.
    fn modern() -> CpuidSnapshot {
        CpuidSnapshot {
            max_leaf: 0x16,
            leaf1_ecx: LEAF1_ECX_OSXSAVE | LEAF1_ECX_AVX | 0x1,
            leaf7_ebx: LEAF7_EBX_AVX2 | (1 << 3) | (1 << 8),
            xcr0: 0b111,
        }
    }

    #[test]
    fn a_modern_cpu_with_an_avx_aware_os_has_avx2() {
        assert!(avx2_usable(modern()));
    }

    #[test]
    fn a_cpu_without_the_avx2_bit_does_not() {
        // Sandy/Ivy Bridge: AVX but no AVX2.
        let s = CpuidSnapshot {
            leaf7_ebx: 0,
            ..modern()
        };
        assert!(!avx2_usable(s));
    }

    #[test]
    fn a_cpu_without_leaf_7_does_not() {
        let s = CpuidSnapshot {
            max_leaf: 6,
            ..modern()
        };
        assert!(!avx2_usable(s));
    }

    #[test]
    fn avx2_needs_avx_and_osxsave() {
        let no_avx = CpuidSnapshot {
            leaf1_ecx: LEAF1_ECX_OSXSAVE,
            ..modern()
        };
        assert!(!avx2_usable(no_avx));
        let no_osxsave = CpuidSnapshot {
            leaf1_ecx: LEAF1_ECX_AVX,
            ..modern()
        };
        assert!(!avx2_usable(no_osxsave));
    }

    #[test]
    fn avx2_needs_the_os_to_save_ymm_state() {
        // A hypervisor that hides AVX state from the guest.
        let sse_only = CpuidSnapshot {
            xcr0: 0b011,
            ..modern()
        };
        assert!(!avx2_usable(sse_only));
        let ymm_without_xmm = CpuidSnapshot {
            xcr0: 0b101,
            ..modern()
        };
        assert!(!avx2_usable(ymm_without_xmm));
    }

    #[test]
    fn an_empty_snapshot_has_no_avx2() {
        assert!(!avx2_usable(CpuidSnapshot::default()));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_cpuid_probe_agrees_with_the_standard_library() {
        assert_eq!(
            cpu_has_avx2(),
            Some(std::arch::is_x86_feature_detected!("avx2"))
        );
    }

    #[cfg(not(target_arch = "x86_64"))]
    #[test]
    fn other_architectures_have_no_answer() {
        assert_eq!(cpu_has_avx2(), None);
    }
}
