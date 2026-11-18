//! R4 SttEngine (section 4.3).
//!
//! Responsibilities: Parakeet TDT 0.6B v3 int8 on the official `sherpa-onnx` crate, CPU EP, one
//! worker thread. Packs audio for cloud ASR.
//!
//! The engine itself (model residency, preload on key-down, the decode of closed VAD segments
//! during speech) is [`crate::stt_engine`] (#46, ADR 0082). This module keeps the R4 interface,
//! the recognizer settings and the local model identity.

use crate::error::CoreResult;
use crate::types::Transcript;

/// Local speech to text. It gets segment IDs from R2, not raw audio from callers.
pub trait SttEngine: Send + Sync {
    /// Load the local model into memory now. Errors are content-free codes
    /// ([`crate::error::CoreError::Unavailable`], for example `stt_model_missing`).
    fn load_model(&self) -> CoreResult<()>;
    /// The transcript of one segment that R2 made (the `stt_transcribe` result). Each
    /// transcript is handed out once.
    fn transcribe(&self, segment_id: &str) -> CoreResult<Transcript>;
}

/// The model of local STT, for `Transcript.model` (CAP-72).
pub const LOCAL_STT_MODEL: &str = "parakeet-tdt-0.6b-v3-int8";
/// The runtime of local STT, for `Transcript.runtime` (CAP-72).
pub const LOCAL_STT_RUNTIME: &str = "sherpa-onnx 2.0.1 (onnxruntime CPU EP)";

/// The ONNX Runtime execution provider of local STT (CAP-48, D78): the CPU EP only.
pub const LOCAL_STT_PROVIDER: &str = "cpu";

/// The sherpa-onnx recognizer settings of local Parakeet (CAP-48, D78).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSttConfig {
    /// Always [`LOCAL_STT_PROVIDER`].
    pub provider: &'static str,
    /// One thread for each physical performance core ([`perf_core_count`]).
    pub num_threads: usize,
    /// The sherpa-onnx model type of Parakeet TDT.
    pub model_type: &'static str,
    /// The decoding method.
    pub decoding_method: &'static str,
}

impl LocalSttConfig {
    /// The settings for this device: CPU EP, one thread per physical P-core.
    pub fn for_this_device() -> Self {
        LocalSttConfig {
            provider: LOCAL_STT_PROVIDER,
            num_threads: perf_core_count(),
            model_type: "nemo_transducer",
            decoding_method: "greedy_search",
        }
    }
}

/// The number of cores in the highest efficiency class, given one class for each physical core
/// (Windows `PROCESSOR_RELATIONSHIP::EfficiencyClass`: a higher class is a faster core). A CPU
/// with one core type reports one class, so all its cores count. Never less than 1.
pub fn p_cores_from_efficiency_classes(classes: &[u8]) -> usize {
    let Some(top) = classes.iter().max() else {
        return 1;
    };
    classes.iter().filter(|c| *c == top).count().max(1)
}

/// The number of physical performance cores of this device (CAP-48).
///
/// macOS: `hw.perflevel0.physicalcpu` (`hw.physicalcpu` on a Mac with one core type).
/// Windows: the physical cores of the highest efficiency class
/// (`GetLogicalProcessorInformationEx`). Other systems (Linux is not a ship target): the
/// available parallelism, which counts logical CPUs (SMT threads), not physical P-cores.
pub fn perf_core_count() -> usize {
    platform::perf_core_count()
        .filter(|n| *n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// The `EfficiencyClass` of each `RelationProcessorCore` record in the buffer that
/// `GetLogicalProcessorInformationEx(RelationProcessorCore, ...)` writes. Each record is
/// `Relationship: u32`, `Size: u32`, then `PROCESSOR_RELATIONSHIP` (`Flags: u8`,
/// `EfficiencyClass: u8`, ...); a core record is 48 bytes on x64, shorter than the 80-byte
/// `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` union. The walk uses each record's own `Size` and
/// stops at a record that is too small or runs past the end. Pure, so it is tested on every OS.
pub fn efficiency_classes_from_slpi(buf: &[u8]) -> Vec<u8> {
    const RELATION_PROCESSOR_CORE: u32 = 0;
    const MIN_RECORD: usize = 10; // the header (8) + Flags + EfficiencyClass
    let mut classes = Vec::new();
    let mut off = 0usize;
    while let Some(head) = buf.get(off..off + 8) {
        let relationship = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let Some(end) = off.checked_add(size) else {
            break;
        };
        if size < MIN_RECORD || end > buf.len() {
            break;
        }
        if relationship == RELATION_PROCESSOR_CORE {
            classes.push(buf[off + 9]);
        }
        off = end;
    }
    classes
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_int, c_void, CStr};

    extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    fn sysctl_int(name: &CStr) -> Option<usize> {
        let mut v: c_int = 0;
        let mut len = std::mem::size_of::<c_int>();
        // SAFETY: `name` is NUL-terminated, `v` and `len` are valid for writes and `len` holds
        // the size of `v`. No new value is set.
        let rc = unsafe {
            sysctlbyname(
                name.as_ptr(),
                (&mut v as *mut c_int).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && v > 0).then_some(v as usize)
    }

    pub(super) fn perf_core_count() -> Option<usize> {
        sysctl_int(c"hw.perflevel0.physicalcpu").or_else(|| sysctl_int(c"hw.physicalcpu"))
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };

    pub(super) fn perf_core_count() -> Option<usize> {
        let mut len: u32 = 0;
        // SAFETY: a null buffer with length 0 asks only for the needed size.
        unsafe {
            GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &mut len)
        };
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` holds `len` bytes and `len` is its size in bytes. The call writes
        // variable-size records; they are parsed as bytes, never as a struct reference.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(
                RelationProcessorCore,
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if ok == 0 {
            return None;
        }
        buf.truncate(len as usize);
        let classes = super::efficiency_classes_from_slpi(&buf);
        (!classes.is_empty()).then(|| super::p_cores_from_efficiency_classes(&classes))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub(super) fn perf_core_count() -> Option<usize> {
        None
    }
}
