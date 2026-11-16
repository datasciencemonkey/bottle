//! R4 SttEngine (section 4.3).
//!
//! Responsibilities: Parakeet TDT 0.6B v3 int8 on the official `sherpa-onnx` crate, CPU EP, one worker thread. Packs audio for cloud ASR.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R4";

/// Local speech to text. It gets segment IDs from R2, not raw audio from callers.
pub trait SttEngine: Send + Sync {
    /// Load the local model into memory.
    fn load_model(&self) -> CoreResult<()>;
    /// Transcribe one segment that R2 made. Returns the text.
    fn transcribe(&self, segment_id: &str) -> CoreResult<String>;
}

/// The R4 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubSttEngine;

impl SttEngine for StubSttEngine {
    fn load_model(&self) -> CoreResult<()> {
        // TODO(#46): model residency and preload on key-down.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn transcribe(&self, _segment_id: &str) -> CoreResult<String> {
        // TODO(#46): streaming VAD decode on one worker thread.
        Err(CoreError::NotImplemented { component: ID })
    }
}

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
/// (`GetLogicalProcessorInformationEx`). Other systems: the available parallelism.
pub fn perf_core_count() -> usize {
    platform::perf_core_count()
        .filter(|n| *n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
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
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
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
        // u64 words keep the records aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buf` holds at least `len` bytes and `len` is its size in bytes.
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
        let base = buf.as_ptr().cast::<u8>();
        let mut classes = Vec::new();
        let mut off = 0usize;
        while off + std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>() <= len as usize {
            // SAFETY: `off` is the start of a record inside the `len` bytes that the call wrote;
            // each record holds its own size.
            let rec = unsafe {
                &*base
                    .add(off)
                    .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
            };
            if rec.Relationship == RelationProcessorCore {
                // SAFETY: the union holds `Processor` for `RelationProcessorCore` records.
                classes.push(unsafe { rec.Anonymous.Processor.EfficiencyClass });
            }
            if rec.Size == 0 {
                break;
            }
            off += rec.Size as usize;
        }
        (!classes.is_empty()).then(|| super::p_cores_from_efficiency_classes(&classes))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub(super) fn perf_core_count() -> Option<usize> {
        None
    }
}
