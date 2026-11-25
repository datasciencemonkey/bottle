//! The macOS system audio backend: a Core Audio process tap (ADR 0103, #107).
//!
//! [`TapHost`] is a [`MicHost`] for [`super::system::SystemAudioDevice`]. Each build makes:
//!
//! 1. a `CATapDescription` with `initStereoGlobalTapButExcludeProcesses:` that taps every
//!    process's output except Bottle's own process (so Bottle's sounds are not recorded), set
//!    private and unmuted (the user still hears the call);
//! 2. the tap (`AudioHardwareCreateProcessTap`);
//! 3. a private aggregate device whose input is the tap (tap auto start) **and** the system's
//!    default output device, set as the aggregate's main (clock) sub-device. Apple's
//!    process-tap recipe and the `AudioCap` sample both build the aggregate this way: a tap
//!    alone has no clock, so its IO proc never runs (review of 2026-11-25, CAP-10c: an earlier
//!    build with a tap-only aggregate opened in 22 ms but delivered zero blocks; `firstSystemMs`
//!    stayed `None`). The output device never plays anything through this aggregate (it is not
//!    in the output scope the app uses); it only lends its clock. The main sub-device has no
//!    drift compensation (review of 2026-11-26, "8"): it defines the clock the aggregate runs
//!    on, so there is nothing to compensate it against; only the tap (a real sub-device, not
//!    the clock) is drift-compensated to that clock.
//!
//!    This reads `kAudioHardwarePropertyDefaultOutputDevice`, not
//!    `kAudioHardwarePropertyDefaultSystemOutputDevice` (review of 2026-11-26, "8"): the tap
//!    captures *every process's* output, and `DefaultOutputDevice` is the device that output
//!    actually goes to today. `DefaultSystemOutputDevice` is only where the OS alert/UI sound
//!    effects go, which a user can point at a different device (System Settings' "play sound
//!    effects through"); using it here would hand the aggregate a clock from a device most
//!    process audio, and the tap, may not even be running on.
//! 4. an IO proc on the aggregate device that hands the tap's float frames to the data
//!    callback. `play` starts the IO proc; dropping the [`TapStream`] stops it and destroys the
//!    IO proc, the aggregate device and the tap, in that order.
//!
//! **Multi-buffer input (review of 2026-11-26, "1").** The output device lent to the aggregate
//! for its clock can itself have input-scope streams (a USB headset's mic, or AirPods in the
//! HFP profile): those streams' buffers reach this aggregate's IO proc too, ahead of the tap's
//! own buffers ("sub-device streams come before the tap streams" in the aggregate's combined
//! buffer list). [`TapHost::build`] counts the output device's own input streams
//! (`kAudioDevicePropertyStreams`, input scope) once, at build time, and [`deliver`] skips
//! exactly that many buffers before it ever touches the tap's audio, so a connected headset's
//! own mic input is never mixed into the system audio track. Each remaining buffer keeps its
//! own channel count (mono per buffer, or one interleaved multi-channel buffer): [`deliver`]
//! never assumes every buffer is mono once more than one is left.
//!
//! **Device change mid-call (review of 2026-11-26, "2").** [`TapHost::build`] also listens for
//! `kAudioHardwarePropertyDefaultOutputDevice` (the system object) and `kAudioDevicePropertyDeviceIsAlive`
//! on both the aggregate and the main sub-device. Any of the three firing calls
//! `on_fault(StreamFault::Invalidated)`, the same signal [`super::system::SystemAudioDevice`]'s
//! worker already uses to rebuild a lost mic stream; the worker allows a bounded number of
//! rebuilds within one open (not just one), each one registering its own fresh listeners on the
//! new aggregate and sub-device IDs. [`TapStream::drop`] removes every listener before it
//! destroys the aggregate, so the aggregate's own teardown is never read back as "the device
//! went away" by this same stream.
//!
//! Permission (CAP-10b): the first start shows the macOS "System Audio Recording" prompt with
//! the `NSAudioCaptureUsageDescription` text of `src-tauri/Info.plist`. There is no preflight
//! API, and Bottle uses no private TCC API. A denied permission gives digital silence, which
//! the recording session reports as `silent_or_denied` after 5 s (CAP-10a).
//!
//! The process tap API is macOS 14.2+; Bottle's minimum is 14.4 (`tauri.conf.json`). When the
//! `CATapDescription` class is missing, the build returns `system_audio_unsupported` and the
//! recording goes on mic-only.
//!
//! The bindings are `objc2-core-audio` 0.3.2 (already in Cargo.lock through cpal, licence
//! Zlib OR Apache-2.0 OR MIT). The code is Bottle's own; the dictionary layout follows Apple's
//! `AudioHardware.h` keys.

use std::ffi::{c_void, CStr};
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::AnyThread;
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioDevicePropertyDeviceIsAlive, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioDevicePropertyTransportType, kAudioDeviceTransportTypeAggregate,
    kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBluetoothLE,
    kAudioDeviceTransportTypeBuiltIn, kAudioDeviceTransportTypeUSB,
    kAudioDeviceTransportTypeVirtual, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput, kAudioObjectSystemObject,
    kAudioSubDeviceUIDKey, kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
    kAudioTapPropertyFormat, AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID,
    AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectAddPropertyListener, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListener, CATapDescription, CATapMuteBehavior,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSMutableDictionary, NSNumber, NSString};

use crate::capture::DeviceError;

use super::{codes, DataFn, DeviceKind, FaultFn, Interleaved, MicHost, StreamDiagnostics};

static INSTANCE: AtomicU32 = AtomicU32::new(0);

/// The Core Audio process tap host. Make it on the system audio thread.
#[derive(Debug, Default)]
pub struct TapHost {
    /// Diagnostics of the last [`TapHost::build`] (success or failure): never a device name.
    last: StreamDiagnostics,
}

impl TapHost {
    /// A host; it opens nothing.
    pub fn new() -> Self {
        Self::default()
    }
}

/// One live tap: the tap, its private aggregate device, the output device it borrowed for a
/// clock, and the IO proc.
pub struct TapStream {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    /// The device lent for the aggregate's clock (0 if the aggregate never got that far).
    output_device: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut Ctx,
    started: AtomicBool,
}

// SAFETY: the IDs are plain numbers; `ctx` is owned by this stream and only freed in `drop`,
// after the IO proc is stopped and destroyed.
unsafe impl Send for TapStream {}

struct Ctx {
    on_data: Mutex<DataFn>,
    on_fault: Mutex<FaultFn>,
    rate: u32,
    float: bool,
    scratch: Mutex<Vec<f32>>,
    /// Buffers before this index in each IO cycle belong to the output device's own
    /// input-scope streams (a USB headset or AirPods in HFP), never the tap (review of
    /// 2026-11-26, "1"): [`deliver`] skips exactly this many before it reads anything.
    skip: usize,
}

fn status(code: i32) -> Result<(), DeviceError> {
    if code == 0 {
        Ok(())
    } else {
        Err(codes::DEVICE_FAILED)
    }
}

fn global(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn scoped(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Bottle's own Core Audio process object, or 0 if Core Audio does not know it yet.
fn own_process_object() -> AudioObjectID {
    let pid: libc::pid_t = std::process::id() as libc::pid_t;
    let address = global(kAudioHardwarePropertyTranslatePIDToProcessObject);
    let mut object: AudioObjectID = 0;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    let code = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            std::mem::size_of::<libc::pid_t>() as u32,
            (&pid as *const libc::pid_t).cast(),
            NonNull::from(&mut size),
            NonNull::from(&mut object).cast(),
        )
    };
    if code == 0 {
        object
    } else {
        0
    }
}

fn tap_format(tap: AudioObjectID) -> Result<AudioStreamBasicDescription, DeviceError> {
    let address = global(kAudioTapPropertyFormat);
    // SAFETY: the struct is plain data; zero is a valid bit pattern.
    let mut format: AudioStreamBasicDescription = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<AudioStreamBasicDescription>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    status(unsafe {
        AudioObjectGetPropertyData(
            tap,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut format).cast(),
        )
    })?;
    Ok(format)
}

fn key(k: &CStr) -> Retained<NSString> {
    NSString::from_str(k.to_str().unwrap_or_default())
}

/// The system's default output device, for the aggregate's clock (see the [module docs](self)).
fn default_output_device() -> Result<AudioObjectID, DeviceError> {
    let address = global(kAudioHardwarePropertyDefaultOutputDevice);
    let mut device: AudioObjectID = 0;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    status(unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut device).cast(),
        )
    })?;
    if device == 0 {
        return Err(codes::DEVICE_FAILED);
    }
    Ok(device)
}

/// The UID of Core Audio device `device`, for the aggregate's sub-device list and its main
/// (clock) sub-device key.
fn device_uid(device: AudioObjectID) -> Result<Retained<NSString>, DeviceError> {
    let address = global(kAudioDevicePropertyDeviceUID);
    let mut out: *mut NSString = std::ptr::null_mut();
    let mut size = std::mem::size_of::<*mut NSString>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    status(unsafe {
        AudioObjectGetPropertyData(
            device,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut out).cast(),
        )
    })?;
    // SAFETY: `kAudioDevicePropertyDeviceUID` hands back one retained `CFString`/`NSString*`
    // (Apple's rule for this property: the caller owns it and must release it); `Retained::
    // from_raw` takes that one retain count, the same as a `Copy`-style Core Foundation call.
    unsafe { Retained::from_raw(out) }.ok_or(codes::DEVICE_FAILED)
}

/// How many input-scope streams `device` has (review of 2026-11-26, "1"): 0 for an
/// output-only device (the common case: built-in speakers, most USB DACs), more for a device
/// that also exposes a mic (a USB headset, AirPods in HFP). `0` on any error: an unreadable
/// count is the same, conservative "nothing to skip" as a device with no input streams.
fn input_stream_count(device: AudioObjectID) -> usize {
    let address = scoped(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    let mut size: u32 = 0;
    // SAFETY: a valid pointer to a local.
    let code = unsafe {
        AudioObjectGetPropertyDataSize(
            device,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if code != 0 {
        return 0;
    }
    size as usize / std::mem::size_of::<AudioObjectID>()
}

/// The real nominal sample rate of `device`, read after it exists (review of 2026-11-26, "11":
/// never assumed from the tap's own format alone, which can differ once the tap joins the
/// aggregate). `None` on any error or a non-positive rate.
fn nominal_sample_rate(device: AudioObjectID) -> Option<f64> {
    let address = global(kAudioDevicePropertyNominalSampleRate);
    let mut rate: f64 = 0.0;
    let mut size = std::mem::size_of::<f64>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    let code = unsafe {
        AudioObjectGetPropertyData(
            device,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut rate).cast(),
        )
    };
    (code == 0 && rate > 0.0).then_some(rate)
}

/// `device`'s transport, classified content-free (review of 2026-11-26, "9"): never its name.
#[allow(non_upper_case_globals)]
fn transport_kind(device: AudioObjectID) -> DeviceKind {
    let address = global(kAudioDevicePropertyTransportType);
    let mut t: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid pointers to locals of the sizes passed.
    let code = unsafe {
        AudioObjectGetPropertyData(
            device,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut t).cast(),
        )
    };
    if code != 0 {
        return DeviceKind::Unknown;
    }
    match t {
        kAudioDeviceTransportTypeBuiltIn => DeviceKind::BuiltIn,
        kAudioDeviceTransportTypeUSB => DeviceKind::Usb,
        kAudioDeviceTransportTypeBluetooth | kAudioDeviceTransportTypeBluetoothLE => {
            DeviceKind::Bluetooth
        }
        kAudioDeviceTransportTypeVirtual | kAudioDeviceTransportTypeAggregate => {
            DeviceKind::Virtual
        }
        _ => DeviceKind::Other,
    }
}

/// The aggregate device description: private, not stacked, the tap as its tap list, and the
/// default output device as its only sub-device **and** its main (clock) sub-device — the
/// clock a tap-only aggregate does not have (see the [module docs](self)). The main sub-device
/// carries no drift compensation key (review of 2026-11-26, "8"): it is the clock itself.
fn aggregate_description(
    tap_uid: &NSString,
    output_uid: &NSString,
    uid: &str,
) -> Retained<NSMutableDictionary<NSString, AnyObject>> {
    let yes = NSNumber::new_bool(true);
    let no = NSNumber::new_bool(false);
    let sub_tap = NSMutableDictionary::<NSString, AnyObject>::new();
    sub_tap.insert(&*key(kAudioSubTapUIDKey), tap_uid as &AnyObject);
    sub_tap.insert(&*key(kAudioSubTapDriftCompensationKey), &yes as &AnyObject);
    let taps = NSArray::from_retained_slice(&[sub_tap]);
    let sub_device = NSMutableDictionary::<NSString, AnyObject>::new();
    sub_device.insert(&*key(kAudioSubDeviceUIDKey), output_uid as &AnyObject);
    let sub_devices = NSArray::from_retained_slice(&[sub_device]);
    let d = NSMutableDictionary::<NSString, AnyObject>::new();
    d.insert(
        &*key(kAudioAggregateDeviceNameKey),
        &NSString::from_str("Bottle call recording") as &AnyObject,
    );
    d.insert(
        &*key(kAudioAggregateDeviceUIDKey),
        &NSString::from_str(uid) as &AnyObject,
    );
    d.insert(&*key(kAudioAggregateDeviceIsPrivateKey), &yes as &AnyObject);
    d.insert(&*key(kAudioAggregateDeviceIsStackedKey), &no as &AnyObject);
    d.insert(
        &*key(kAudioAggregateDeviceTapAutoStartKey),
        &yes as &AnyObject,
    );
    d.insert(&*key(kAudioAggregateDeviceTapListKey), &taps as &AnyObject);
    d.insert(
        &*key(kAudioAggregateDeviceSubDeviceListKey),
        &sub_devices as &AnyObject,
    );
    d.insert(
        &*key(kAudioAggregateDeviceMainSubDeviceKey),
        output_uid as &AnyObject,
    );
    d
}

/// Add a fault listener on `object`/`address`, with `ctx` as its client data. Errors are not
/// fatal to the build (review of 2026-11-26, "2"): a missed notification just means a device
/// change is caught later, by the next tick or a fatal stream error, not that the tap fails to
/// open. Returns whether it was actually added, so [`TapStream::drop`] only removes what was
/// added — though removing one that was not is itself harmless (Core Audio answers an error,
/// which this file already ignores everywhere else it tears a stream down).
fn listen(object: AudioObjectID, address: &AudioObjectPropertyAddress, ctx: *mut Ctx) -> bool {
    // SAFETY: a valid address pointer; `ctx` outlives every listener (removed in `drop` before
    // it is freed).
    let code = unsafe {
        AudioObjectAddPropertyListener(
            object,
            NonNull::from(address),
            Some(fault_listener),
            ctx.cast(),
        )
    };
    code == 0
}

fn unlisten(object: AudioObjectID, address: &AudioObjectPropertyAddress, ctx: *mut Ctx) {
    // SAFETY: a valid address pointer; removing a listener that was never added (or already
    // removed) is a documented no-op error from Core Audio, not undefined behaviour.
    let _ = unsafe {
        AudioObjectRemovePropertyListener(
            object,
            NonNull::from(address),
            Some(fault_listener),
            ctx.cast(),
        )
    };
}

/// The two addresses [`TapStream`] listens on for a "something about this device changed"
/// signal (review of 2026-11-26, "2"): the default output device itself, and whether a given
/// device object is still alive. Both map to [`crate::audio_device::StreamFault::Invalidated`].
fn default_output_address() -> AudioObjectPropertyAddress {
    global(kAudioHardwarePropertyDefaultOutputDevice)
}

fn is_alive_address() -> AudioObjectPropertyAddress {
    global(kAudioDevicePropertyDeviceIsAlive)
}

impl MicHost for TapHost {
    type Stream = TapStream;

    fn build(&mut self, on_data: DataFn, on_fault: FaultFn) -> Result<TapStream, DeviceError> {
        if AnyClass::get(c"CATapDescription").is_none() {
            return Err(codes::SYSTEM_AUDIO_UNSUPPORTED);
        }
        let own = own_process_object();
        let excluded: Retained<NSArray<NSNumber>> = if own == 0 {
            NSArray::new()
        } else {
            NSArray::from_retained_slice(&[NSNumber::new_u32(own)])
        };
        // SAFETY: plain Objective-C calls on a fresh description.
        let desc = unsafe {
            let desc = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &excluded,
            );
            desc.setName(&NSString::from_str("Bottle call recording"));
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::Unmuted);
            desc
        };
        let mut tap: AudioObjectID = 0;
        // SAFETY: a valid description and out pointer.
        status(unsafe { AudioHardwareCreateProcessTap(Some(&desc), &mut tap) })?;
        let mut stream = TapStream {
            tap,
            aggregate: 0,
            output_device: 0,
            proc_id: None,
            ctx: std::ptr::null_mut(),
            started: AtomicBool::new(false),
        };
        // From here a failure drops `stream`, which destroys what exists.
        let format = tap_format(tap)?;
        // SAFETY: plain Objective-C calls.
        let tap_uid = unsafe { desc.UUID().UUIDString() };
        // The aggregate needs a real device for its clock, or its IO proc never runs (see the
        // module docs): the default output device lends it, but plays nothing through this
        // aggregate. No default output device at all (review "9") is its own, distinct error.
        let output_device = match default_output_device() {
            Ok(d) => d,
            Err(_) => {
                self.last = StreamDiagnostics {
                    device_kind: DeviceKind::None,
                    ..StreamDiagnostics::default()
                };
                return Err(codes::NO_OUTPUT_DEVICE);
            }
        };
        stream.output_device = output_device;
        let output_uid = device_uid(output_device)?;
        // Review "1": buffers of the output device's own input streams land before the tap's
        // in the aggregate's IO proc buffer list.
        let skip = input_stream_count(output_device);
        let uid = format!(
            "app.bottle.recording.tap.{}.{}",
            std::process::id(),
            INSTANCE.fetch_add(1, Ordering::Relaxed)
        );
        let dict = aggregate_description(&tap_uid, &output_uid, &uid);
        // SAFETY: NSDictionary is toll-free bridged to CFDictionary.
        let cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&dict) as *const CFDictionary) };
        let mut aggregate: AudioObjectID = 0;
        // SAFETY: a valid dictionary and out pointer.
        status(unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut aggregate)) })?;
        stream.aggregate = aggregate;
        // Review "11": the real rate, read now that the aggregate exists, not the tap's own
        // pre-aggregation format alone.
        let rate = nominal_sample_rate(aggregate).unwrap_or(format.mSampleRate);
        let rate = rate.round();
        self.last = StreamDiagnostics {
            sample_rate_hz: if rate >= 1.0 && rate <= f64::from(u32::MAX) {
                rate as u32
            } else {
                48_000
            },
            device_kind: transport_kind(output_device),
            rebuilds: 0,
        };
        let ctx = Box::into_raw(Box::new(Ctx {
            on_data: Mutex::new(on_data),
            on_fault: Mutex::new(on_fault),
            rate: self.last.sample_rate_hz,
            float: format.mFormatFlags & kAudioFormatFlagIsFloat != 0
                && format.mBitsPerChannel == 32,
            scratch: Mutex::new(Vec::new()),
            skip,
        }));
        stream.ctx = ctx;
        // Review "2": a device change mid-call must reach `on_fault`, the same signal a fatal
        // stream error already uses, so the worker rebuilds instead of silently going quiet.
        // Each listener's own failure to register is not fatal to the build (see `listen`).
        listen(
            kAudioObjectSystemObject as AudioObjectID,
            &default_output_address(),
            ctx,
        );
        listen(aggregate, &is_alive_address(), ctx);
        listen(output_device, &is_alive_address(), ctx);
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: `ctx` lives until the IO proc is destroyed in `TapStream::drop`.
        status(unsafe {
            AudioDeviceCreateIOProcID(
                aggregate,
                Some(io_proc),
                ctx.cast(),
                NonNull::from(&mut proc_id),
            )
        })?;
        stream.proc_id = proc_id;
        Ok(stream)
    }

    fn play(&mut self, stream: &TapStream) -> Result<(), DeviceError> {
        // SAFETY: a live aggregate device and IO proc.
        status(unsafe { AudioDeviceStart(stream.aggregate, stream.proc_id) })?;
        // `started` only steers the stop in drop; a start that failed needs no stop.
        stream.started.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn diagnostics(&self) -> StreamDiagnostics {
        self.last
    }
}

/// The raw IDs one [`TapStream`] drop needs, with no borrow of the stream itself (review of
/// 2026-11-26, "7"): plain numbers and one raw pointer, so the potentially slow part of the
/// teardown — `AudioDeviceStop` (which can take far longer than this app ever bounds its own
/// waits to, when the IO proc never ran), then destroying the IO proc, the aggregate and the
/// tap, then freeing `ctx` — can run on its own thread, never the one that dropped the stream.
/// The listeners come off synchronously in [`TapStream::drop`] itself, before this exists: that
/// part is fast and must never race the aggregate's own destruction.
struct Teardown {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut Ctx,
    started: bool,
}

// SAFETY: every field is a plain Core Audio object ID, a function pointer, or `ctx`, which
// `TapStream::drop` hands off exactly once and never touches again (it does not even read it
// again itself): nothing here aliases with another thread.
unsafe impl Send for Teardown {}

impl Teardown {
    fn run(self) {
        // SAFETY: each ID was made by the stream that handed off this `Teardown`; the stop
        // waits for a running IO proc, so `ctx` is free of readers when it drops.
        unsafe {
            if self.aggregate != 0 && self.proc_id.is_some() {
                if self.started {
                    let _ = AudioDeviceStop(self.aggregate, self.proc_id);
                }
                let _ = AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            }
            if self.aggregate != 0 {
                let _ = AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            if self.tap != 0 {
                let _ = AudioHardwareDestroyProcessTap(self.tap);
            }
            if !self.ctx.is_null() {
                drop(Box::from_raw(self.ctx));
            }
        }
    }
}

impl Drop for TapStream {
    fn drop(&mut self) {
        // The listeners go first, synchronously, before anything else is touched (review
        // "2"): fast (an in-process un-registration, not a device call), and this way this
        // stream's own teardown is never read back by itself as "the device went away", even
        // if the slower teardown below never gets to run at all (a failed thread spawn).
        if !self.ctx.is_null() {
            unlisten(
                kAudioObjectSystemObject as AudioObjectID,
                &default_output_address(),
                self.ctx,
            );
            if self.aggregate != 0 {
                unlisten(self.aggregate, &is_alive_address(), self.ctx);
            }
            if self.output_device != 0 {
                unlisten(self.output_device, &is_alive_address(), self.ctx);
            }
        }
        let teardown = Teardown {
            tap: self.tap,
            aggregate: self.aggregate,
            proc_id: self.proc_id,
            ctx: self.ctx,
            started: self.started.load(Ordering::SeqCst),
        };
        // This stream owns nothing else once this returns: no double-free, no double-stop,
        // even though the real work has not run yet.
        self.ctx = std::ptr::null_mut();
        self.aggregate = 0;
        self.tap = 0;
        self.proc_id = None;
        // Review "7": run the possibly slow part off this thread, so whatever dropped this
        // stream (on macOS, the `bottle-sysaudio` worker, about to read its very next message)
        // never waits behind a slow `AudioDeviceStop`. If the thread cannot be spawned (the OS
        // itself is out of resources), `teardown` simply drops here instead of running — this
        // one aggregate and tap are not explicitly destroyed (the OS still reclaims them when
        // the process exits) and `ctx`'s small buffer is not freed; nothing unsafe, no token,
        // no audio, and the listeners are already off either way.
        let _ = std::thread::Builder::new()
            .name("bottle-sysaudio-teardown".into())
            .spawn(move || teardown.run());
    }
}

unsafe extern "C-unwind" fn fault_listener(
    _object: AudioObjectID,
    _n: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    client: *mut c_void,
) -> i32 {
    if client.is_null() {
        return 0;
    }
    // SAFETY: `client` is the `Ctx` of a live stream; the listener is removed in `drop` before
    // `ctx` is freed, so it never fires on a dangling pointer.
    let ctx = unsafe { &*(client as *const Ctx) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut f) = ctx.on_fault.try_lock() {
            f(crate::audio_device::StreamFault::Invalidated);
        }
    }));
    0
}

unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    if client.is_null() {
        return 0;
    }
    // SAFETY: `client` is the `Ctx` of a live stream (see `TapStream::drop`).
    let ctx = unsafe { &*(client as *const Ctx) };
    // A panic must not unwind into Core Audio.
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Core Audio passes a valid buffer list for the IO cycle.
        unsafe { deliver(ctx, input.as_ref()) }
    }));
    0
}

/// Hand one IO cycle's input to the data callback, interleaving non-interleaved buffers.
/// `ctx.skip` buffers come first from the output device's own input streams (review of
/// 2026-11-26, "1"), never the tap's audio: they are dropped before anything else runs, and
/// every buffer kept after them is read by its own `mNumberChannels`, not assumed mono.
unsafe fn deliver(ctx: &Ctx, list: &AudioBufferList) {
    if !ctx.float {
        return;
    }
    let n = list.mNumberBuffers as usize;
    if n <= ctx.skip {
        // Nothing of the tap's own audio in this cycle (or a format Bottle did not expect).
        return;
    }
    // SAFETY: the list holds `mNumberBuffers` buffers in a row.
    let all = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
    let buffers = &all[ctx.skip..];
    let Ok(mut on_data) = ctx.on_data.try_lock() else {
        return;
    };
    if buffers.len() == 1 {
        let b = &buffers[0];
        if b.mData.is_null() {
            return;
        }
        let len = b.mDataByteSize as usize / 4;
        // SAFETY: the buffer holds `mDataByteSize` bytes of f32 samples.
        let samples = unsafe { std::slice::from_raw_parts(b.mData as *const f32, len) };
        on_data(Interleaved {
            samples,
            channels: u16::try_from(b.mNumberChannels.max(1)).unwrap_or(1),
            sample_rate: ctx.rate,
        });
        return;
    }
    // More than one buffer left: each keeps its own channel count (one or more), never
    // assumed mono. Frames are the shortest of what each buffer actually holds.
    let frames = buffers
        .iter()
        .map(|b| {
            if b.mData.is_null() {
                0
            } else {
                let ch = b.mNumberChannels.max(1) as usize;
                (b.mDataByteSize as usize / 4) / ch
            }
        })
        .min()
        .unwrap_or(0);
    let total_channels: usize = buffers
        .iter()
        .map(|b| b.mNumberChannels.max(1) as usize)
        .sum();
    let Ok(mut scratch) = ctx.scratch.try_lock() else {
        return;
    };
    scratch.clear();
    for f in 0..frames {
        for b in buffers {
            let ch = b.mNumberChannels.max(1) as usize;
            if b.mData.is_null() {
                scratch.extend(std::iter::repeat_n(0.0f32, ch));
                continue;
            }
            // SAFETY: `f` is below each buffer's own frame count, `ch` is this buffer's own
            // channel count.
            let base = unsafe { (b.mData as *const f32).add(f * ch) };
            for c in 0..ch {
                scratch.push(unsafe { *base.add(c) });
            }
        }
    }
    on_data(Interleaved {
        samples: &scratch,
        channels: u16::try_from(total_channels).unwrap_or(u16::MAX),
        sample_rate: ctx.rate,
    });
}

/// The macOS permission asks of ADR 0103 (see [`crate::recording_capture::permissions`]).
#[derive(Debug, Default)]
pub struct MacPermissionAsker;

impl crate::recording_capture::permissions::PermissionAsker for MacPermissionAsker {
    fn ask_mic(&self) {
        use objc2_avf_audio::AVAudioApplication;
        // Shows the Microphone prompt only if macOS has not asked yet; the answer is read
        // later by the mic itself (a denied mic gives the CAP-17b fix-it card).
        let block = block2::RcBlock::new(|_granted: objc2::runtime::Bool| {});
        // SAFETY: a class method with a valid block.
        unsafe { AVAudioApplication::requestRecordPermissionWithCompletionHandler(&block) };
    }

    fn ask_system_audio(&self) {
        // A short tap start shows the System Audio Recording prompt (no preflight API).
        // Off the main thread; the tap is gone after 500 ms.
        let _ = std::thread::Builder::new()
            .name("bottle-sysaudio-ask".into())
            .spawn(|| {
                let mut host = TapHost::new();
                let on_data: DataFn = Box::new(|_| {});
                let on_fault: FaultFn = Box::new(|_| {});
                if let Ok(stream) = host.build(on_data, on_fault) {
                    let _ = host.play(&stream);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    drop(stream);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_audio::kAudioSubDeviceDriftCompensationKey;
    use std::sync::Arc;

    /// The aggregate device recipe fix (review of 2026-11-25, CAP-10c; and of 2026-11-26,
    /// "8" and "12"): the aggregate must carry the output device as a real sub-device, set as
    /// the main (clock) sub-device, next to the tap — not the tap alone, and with no drift
    /// compensation on that main sub-device. No Core Audio HAL call runs here (no hardware
    /// needed): this only checks the dictionary this app builds, against Apple's documented
    /// process-tap recipe and the `AudioCap` sample.
    #[test]
    fn aggregate_description_gives_the_aggregate_a_real_clock() {
        let tap_uid = NSString::from_str("tap-uid-fake");
        let output_uid = NSString::from_str("output-uid-fake");
        let dict = aggregate_description(&tap_uid, &output_uid, "app.bottle.recording.tap.test");

        // The tap is still the (only) tap list entry.
        let taps = dict
            .objectForKey(&*key(kAudioAggregateDeviceTapListKey))
            .expect("tap list key");
        let taps: &NSArray<AnyObject> =
            unsafe { &*(Retained::as_ptr(&taps) as *const NSArray<AnyObject>) };
        assert_eq!(taps.len(), 1);

        // The output device is a sub-device: this is the clock the tap alone does not have.
        let subs = dict
            .objectForKey(&*key(kAudioAggregateDeviceSubDeviceListKey))
            .expect("sub-device list key (the fix: a tap-only aggregate has no clock)");
        let subs: &NSArray<AnyObject> =
            unsafe { &*(Retained::as_ptr(&subs) as *const NSArray<AnyObject>) };
        assert_eq!(subs.len(), 1, "exactly the output device, nothing else");
        // "12": the sub-device dictionary itself must carry the output device's UID, not just
        // the top-level main-sub-device key (a dict with the wrong UID in the sub-device list
        // but the right one in `kAudioAggregateDeviceMainSubDeviceKey` would still "work" by
        // accident on some systems and hide a real bug here).
        let sub0 = subs.objectAtIndex(0);
        let sub0_dict: &NSMutableDictionary<NSString, AnyObject> =
            unsafe { &*(&raw const *sub0 as *const NSMutableDictionary<NSString, AnyObject>) };
        let sub_uid = sub0_dict
            .objectForKey(&*key(kAudioSubDeviceUIDKey))
            .expect("sub-device UID key");
        let sub_uid: &NSString = unsafe { &*(Retained::as_ptr(&sub_uid) as *const NSString) };
        assert_eq!(sub_uid.to_string(), "output-uid-fake");
        // "8": the main (clock) sub-device itself is never drift-compensated.
        assert!(
            sub0_dict
                .objectForKey(&*key(kAudioSubDeviceDriftCompensationKey))
                .is_none(),
            "the clock sub-device must not carry its own drift compensation key"
        );

        // The main (clock) sub-device is that same output device's UID.
        let main = dict
            .objectForKey(&*key(kAudioAggregateDeviceMainSubDeviceKey))
            .expect("main sub-device key");
        let main: &NSString = unsafe { &*(Retained::as_ptr(&main) as *const NSString) };
        assert_eq!(main.to_string(), "output-uid-fake");

        // Still private, not stacked, tap auto-start on: unchanged from before the fix.
        let is_private = dict
            .objectForKey(&*key(kAudioAggregateDeviceIsPrivateKey))
            .expect("private key");
        let is_private: &NSNumber = unsafe { &*(Retained::as_ptr(&is_private) as *const NSNumber) };
        assert!(is_private.as_bool());
    }

    /// Build a fake `AudioBufferList` with `channels.len()` trailing buffers, each with
    /// `channels[i]` interleaved channels of `frames` constant-valued samples. No Core Audio
    /// call: a hand-built struct of the same `#[repr(C)]` layout Core Audio itself uses, so
    /// `deliver` reads it exactly as it would read a real IO cycle's buffer list.
    fn fake_list(channels: &[(u32, f32)], frames: usize) -> (Vec<Vec<f32>>, Vec<AudioBuffer2>) {
        let mut backing = Vec::new();
        let mut bufs = Vec::new();
        for &(ch, value) in channels {
            let samples = vec![value; frames * ch as usize];
            bufs.push(AudioBuffer2 {
                m_number_channels: ch,
                m_data_byte_size: (samples.len() * 4) as u32,
                m_data: samples.as_ptr() as *mut c_void,
            });
            backing.push(samples);
        }
        (backing, bufs)
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AudioBuffer2 {
        m_number_channels: u32,
        m_data_byte_size: u32,
        m_data: *mut c_void,
    }

    /// Review of 2026-11-26, "1": a 1-channel buffer (the output device's own mic, skipped)
    /// ahead of a 2-channel buffer (the tap's real stereo audio) must deliver only the tap's
    /// samples, at its own channel count — never mixed with the mic buffer, never forced mono.
    #[test]
    fn deliver_skips_the_output_devices_own_input_buffer_and_keeps_the_taps_own_channel_count() {
        let (_backing, bufs) = fake_list(&[(1, 0.9), (2, 0.25)], 4);
        #[repr(C)]
        struct List2 {
            m_number_buffers: u32,
            m_buffers: [AudioBuffer2; 2],
        }
        let list = List2 {
            m_number_buffers: 2,
            m_buffers: [bufs[0], bufs[1]],
        };
        type Got = Arc<Mutex<Option<(Vec<f32>, u16)>>>;
        let got: Got = Arc::new(Mutex::new(None));
        let got2 = Arc::clone(&got);
        let ctx = Ctx {
            on_data: Mutex::new(Box::new(move |i: Interleaved<'_>| {
                *got2.lock().unwrap() = Some((i.samples.to_vec(), i.channels));
            })),
            on_fault: Mutex::new(Box::new(|_| {})),
            rate: 48_000,
            float: true,
            scratch: Mutex::new(Vec::new()),
            skip: 1,
        };
        // SAFETY: `list` is a real, well-formed `AudioBufferList` of this process's own
        // backing arrays, cast through the same `#[repr(C)]` layout Core Audio uses.
        unsafe { deliver(&ctx, &*(&raw const list as *const AudioBufferList)) };
        let (samples, channels) = got.lock().unwrap().take().expect("on_data ran once");
        assert_eq!(
            channels, 2,
            "only the tap's own (2) channels, not mixed with the mic's 1"
        );
        assert_eq!(samples, vec![0.25; 8], "the mic buffer's 0.9 never appears");
    }

    /// With nothing left after `skip` (an all-input cycle, or a buffer count Bottle did not
    /// expect), `deliver` must not call `on_data` at all, and must not panic.
    #[test]
    fn deliver_with_nothing_after_skip_calls_on_data_zero_times() {
        let (_backing, bufs) = fake_list(&[(1, 0.9)], 4);
        #[repr(C)]
        struct List1 {
            m_number_buffers: u32,
            m_buffers: [AudioBuffer2; 1],
        }
        let list = List1 {
            m_number_buffers: 1,
            m_buffers: [bufs[0]],
        };
        let calls = Arc::new(Mutex::new(0u32));
        let calls2 = Arc::clone(&calls);
        let ctx = Ctx {
            on_data: Mutex::new(Box::new(move |_| {
                *calls2.lock().unwrap() += 1;
            })),
            on_fault: Mutex::new(Box::new(|_| {})),
            rate: 48_000,
            float: true,
            scratch: Mutex::new(Vec::new()),
            skip: 1,
        };
        unsafe { deliver(&ctx, &*(&raw const list as *const AudioBufferList)) };
        assert_eq!(*calls.lock().unwrap(), 0);
    }
}
