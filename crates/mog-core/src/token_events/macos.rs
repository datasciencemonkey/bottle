//! The macOS sleep and lock source (#465, ADR 0076). Compiled on macOS only.
//!
//! Sleep: `NSWorkspaceWillSleepNotification` on the workspace notification center. AppKit
//! posts it on the main thread, and the block runs on the posting thread.
//! Lock: the `com.apple.screenIsLocked` distributed notification, delivered on the main run
//! loop. Both blocks only call the sink with the decoded event.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSObjectProtocol,
    NSString,
};

use super::{mac_event, SessionEventSource, SessionSink, SessionWatch, MAC_SCREEN_LOCKED};
use crate::error::CoreResult;

/// The macOS source.
pub(super) struct MacSessionEvents;

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// The two observers. Dropping this removes them, so no block runs after that.
struct Observers {
    workspace: Retained<NSNotificationCenter>,
    distributed: Retained<NSDistributedNotificationCenter>,
    sleep: Observer,
    lock: Observer,
}

// SAFETY: the notification centers are thread-safe (`Send + Sync` in objc2). The observer
// tokens are opaque objects that are only passed back to `removeObserver:`, which may be
// called from any thread.
unsafe impl Send for Observers {}

impl Drop for Observers {
    fn drop(&mut self) {
        // SAFETY: each token came from `addObserverForName:` on the same center.
        unsafe {
            self.workspace
                .removeObserver(AsRef::<AnyObject>::as_ref(&*self.sleep));
            self.distributed
                .removeObserver(AsRef::<AnyObject>::as_ref(&*self.lock));
        }
    }
}

/// A block that decodes the notification name and calls the sink for a sleep or lock name.
fn block(sink: SessionSink) -> RcBlock<dyn Fn(NonNull<NSNotification>)> {
    RcBlock::new(move |n: NonNull<NSNotification>| {
        // SAFETY: the center passes a valid notification for the duration of the call.
        let name = unsafe { n.as_ref() }.name().to_string();
        if let Some(event) = mac_event(&name) {
            sink(event);
        }
    })
}

impl SessionEventSource for MacSessionEvents {
    fn watch(&self, sink: SessionSink) -> CoreResult<SessionWatch> {
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        let lock_name = NSString::from_str(MAC_SCREEN_LOCKED);
        let sleep_block = block(sink.clone());
        let lock_block = block(sink);
        // SAFETY: static AppKit name, no object filter, no queue (the block runs on the
        // posting thread). The blocks are `Send + Sync` (they hold only the sink).
        let (sleep, lock) = unsafe {
            (
                workspace.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceWillSleepNotification),
                    None,
                    None,
                    &sleep_block,
                ),
                distributed.addObserverForName_object_queue_usingBlock(
                    Some(&lock_name),
                    None,
                    None,
                    &lock_block,
                ),
            )
        };
        let observers = Observers {
            workspace,
            distributed,
            sleep,
            lock,
        };
        Ok(SessionWatch::on_drop(move || drop(observers)))
    }
}
