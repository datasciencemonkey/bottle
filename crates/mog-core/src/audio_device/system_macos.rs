//! The macOS system audio backend: a Core Audio process tap (ADR 0103, #107).
//!
//! [`TapHost`] is a [`MicHost`] for [`super::system::SystemAudioDevice`]. Each build makes:
//!
//! 1. a `CATapDescription` with `initStereoGlobalTapButExcludeProcesses:` that taps every
//!    process's output except Bottle's own process (so Bottle's sounds are not recorded), set
//!    private and unmuted (the user still hears the call);
//! 2. the tap (`AudioHardwareCreateProcessTap`);
//! 3. a private aggregate device whose input is the tap (tap auto start, drift compensation
//!    on) **and** the system's default output device, set as the aggregate's main (clock)
//!    sub-device. Apple's process-tap recipe and the `AudioCap` sample both build the
//!    aggregate this way: a tap alone has no clock, so its IO proc never runs (review of
//!    2026-11-25, CAP-10c: an earlier build with a tap-only aggregate opened in 22 ms but
//!    delivered zero blocks; `firstSystemMs` stayed `None`). The output device never plays
//!    anything through this aggregate (it is not in the output scope the app uses); it only
//!    lends its clock.
//! 4. an IO proc on the aggregate device that hands the tap's float frames to the data
//!    callback. `play` starts the IO proc; dropping the [`TapStream`] stops it and destroys the
//!    IO proc, the aggregate device and the tap, in that order.
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
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyTranslatePIDToProcessObject,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    kAudioSubDeviceDriftCompensationKey, kAudioSubDeviceUIDKey, kAudioSubTapDriftCompensationKey,
    kAudioSubTapUIDKey, kAudioTapPropertyFormat, AudioDeviceCreateIOProcID,
    AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSMutableDictionary, NSNumber, NSString};

use crate::capture::DeviceError;

use super::{codes, DataFn, FaultFn, Interleaved, MicHost};

static INSTANCE: AtomicU32 = AtomicU32::new(0);

/// The Core Audio process tap host. Make it on the system audio thread.
#[derive(Debug, Default)]
pub struct TapHost;

impl TapHost {
    /// A host; it opens nothing.
    pub fn new() -> Self {
        Self
    }
}

/// One live tap: the tap, its private aggregate device and the IO proc.
pub struct TapStream {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut Ctx,
    started: AtomicBool,
}

// SAFETY: the IDs are plain numbers; `ctx` is owned by this stream and only freed in `drop`,
// after the IO proc is stopped and destroyed.
unsafe impl Send for TapStream {}

struct Ctx {
    on_data: Mutex<DataFn>,
    rate: u32,
    float: bool,
    scratch: Mutex<Vec<f32>>,
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

/// The aggregate device description: private, not stacked, the tap as its tap list, and the
/// default output device as its only sub-device **and** its main (clock) sub-device — the
/// clock a tap-only aggregate does not have (see the [module docs](self)).
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
    sub_device.insert(
        &*key(kAudioSubDeviceDriftCompensationKey),
        &yes as &AnyObject,
    );
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

impl MicHost for TapHost {
    type Stream = TapStream;

    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<TapStream, DeviceError> {
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
        // aggregate.
        let output_device = default_output_device()?;
        let output_uid = device_uid(output_device)?;
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
        let rate = format.mSampleRate.round();
        let ctx = Box::into_raw(Box::new(Ctx {
            on_data: Mutex::new(on_data),
            rate: if rate >= 1.0 && rate <= f64::from(u32::MAX) {
                rate as u32
            } else {
                48_000
            },
            float: format.mFormatFlags & kAudioFormatFlagIsFloat != 0
                && format.mBitsPerChannel == 32,
            scratch: Mutex::new(Vec::new()),
        }));
        stream.ctx = ctx;
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
}

impl Drop for TapStream {
    fn drop(&mut self) {
        // SAFETY: each ID was made by this stream; the stop waits for a running IO proc, so
        // `ctx` is free of readers when it drops.
        unsafe {
            if self.aggregate != 0 && self.proc_id.is_some() {
                if self.started.load(Ordering::SeqCst) {
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
unsafe fn deliver(ctx: &Ctx, list: &AudioBufferList) {
    if !ctx.float {
        return;
    }
    let n = list.mNumberBuffers as usize;
    if n == 0 {
        return;
    }
    // SAFETY: the list holds `mNumberBuffers` buffers in a row.
    let buffers = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
    let Ok(mut on_data) = ctx.on_data.try_lock() else {
        return;
    };
    if n == 1 {
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
    // Non-interleaved: one mono buffer per channel.
    let frames = buffers
        .iter()
        .map(|b| {
            if b.mData.is_null() {
                0
            } else {
                b.mDataByteSize as usize / 4
            }
        })
        .min()
        .unwrap_or(0);
    let Ok(mut scratch) = ctx.scratch.try_lock() else {
        return;
    };
    scratch.clear();
    for f in 0..frames {
        for b in buffers {
            // SAFETY: `f` is below each buffer's frame count.
            scratch.push(unsafe { *(b.mData as *const f32).add(f) });
        }
    }
    on_data(Interleaved {
        samples: &scratch,
        channels: u16::try_from(n).unwrap_or(u16::MAX),
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

    /// The aggregate device recipe fix (review of 2026-11-25, CAP-10c): the aggregate must
    /// carry the output device as a real sub-device, set as the main (clock) sub-device, next
    /// to the tap — not the tap alone. No Core Audio HAL call runs here (no hardware needed):
    /// this only checks the dictionary this app builds, against Apple's documented
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
}
