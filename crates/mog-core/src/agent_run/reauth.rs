//! The "sign in, then retry" rule (ADR 0122 amendment, owner decision 2026-11-24; generalized to
//! every call, round 4).
//!
//! The owner's rule (verbatim): "In the absence of a sign-in, or if the sign-in had expired,
//! refresh the sign-in. The user is always going to be watching it, so it doesn't matter. You
//! can just open up the browser to refresh the sign-in using the U2M OAuth."
//!
//! A `signed_out` `ask_genie` call never widens the home-first rule on its own: the workspace has
//! not said it lacks data, its token has merely expired. Instead the orchestrator host starts
//! that workspace's browser sign-in (the same U2M OAuth flow as `profile_reauth`, "Sign in…" in
//! Settings → Workspace) and retries the call once. [`ReauthService`] is that flow, as the
//! orchestrator sees it: no new OAuth implementation, no token in Rust's hands beyond what
//! `profile_reauth` already keeps.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};

use crate::error::{CoreError, CoreResult};

/// Starts a profile's browser sign-in and blocks until it ends, fails, is cancelled or times
/// out, with whatever bound the underlying flow already has. Implementations must dedupe: a
/// second call for a profile that is already signing in must wait for that attempt's result
/// instead of starting a second browser window ([`DedupReauth`] does this for any inner flow).
pub trait ReauthService: Send + Sync {
    fn reauth(&self, profile: &str) -> CoreResult<()>;

    /// Cancel `profile`'s in-flight sign-in, if it has one, so a Stop, the turn's hard deadline
    /// or a Wipe never leaves the worker thread blocked well past any of them (ADR 0122
    /// amendment, round 4). A no-op default: a service with no sign-in ever in flight (such as
    /// [`NoReauth`]) has nothing to cancel.
    fn cancel(&self, profile: &str) {
        let _ = profile;
    }
}

/// A `ReauthService` with no real flow: refuses every call. The default for a build with no
/// sign-in flow wired yet, so a missing dependency is a clean refusal, not a panic.
pub struct NoReauth;

impl ReauthService for NoReauth {
    fn reauth(&self, _profile: &str) -> CoreResult<()> {
        Err(CoreError::IpcRefused {
            code: "host_failed",
        })
    }
}

#[derive(Default)]
struct Slot {
    /// `None` while the sign-in is still running.
    done: Mutex<Option<Result<(), &'static str>>>,
    cv: Condvar,
}

/// Wraps a `reauth` flow with the owner's "one sign-in at a time per profile" rule: a second
/// caller for a profile that is already signing in waits for the first's result, so only one
/// browser window ever opens for that profile at once. `cancel_fn` reaches the same flow's own
/// cancel (round 4): `DedupReauth` itself holds no running state to cancel, so it only forwards.
pub struct DedupReauth<F, C> {
    run: F,
    cancel_fn: C,
    inflight: Mutex<HashMap<String, Arc<Slot>>>,
}

impl<F, C> DedupReauth<F, C>
where
    F: Fn(&str) -> CoreResult<()> + Send + Sync,
    C: Fn(&str) + Send + Sync,
{
    pub fn new(run: F, cancel_fn: C) -> Self {
        Self {
            run,
            cancel_fn,
            inflight: Mutex::new(HashMap::new()),
        }
    }
}

/// The error code of a panic inside the wrapped flow: the slot still fills (with this code)
/// instead of staying `None` forever, so no waiter blocks on a runner that never finishes.
const PANICKED: &str = "host_failed";

impl<F, C> ReauthService for DedupReauth<F, C>
where
    F: Fn(&str) -> CoreResult<()> + Send + Sync,
    C: Fn(&str) + Send + Sync,
{
    fn reauth(&self, profile: &str) -> CoreResult<()> {
        let (slot, is_runner) = {
            let mut m = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(existing) = m.get(profile) {
                (Arc::clone(existing), false)
            } else {
                let slot = Arc::new(Slot::default());
                m.insert(profile.to_owned(), Arc::clone(&slot));
                (slot, true)
            }
        };
        if is_runner {
            // A panic of `run` must still fill the slot: an unwinding runner that never reaches
            // the line below would otherwise leave every waiter blocked on `slot.cv` forever.
            let result = panic::catch_unwind(AssertUnwindSafe(|| (self.run)(profile)))
                .unwrap_or(Err(CoreError::IpcRefused { code: PANICKED }))
                .map_err(|e| e.code());
            *slot.done.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
            slot.cv.notify_all();
            self.inflight
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(profile);
            return result.map_err(|code| CoreError::IpcRefused { code });
        }
        let mut done = slot.done.lock().unwrap_or_else(|p| p.into_inner());
        while done.is_none() {
            done = slot.cv.wait(done).unwrap_or_else(|p| p.into_inner());
        }
        done.unwrap().map_err(|code| CoreError::IpcRefused { code })
    }

    fn cancel(&self, profile: &str) {
        (self.cancel_fn)(profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    /// A `DedupReauth` whose cancel side does nothing, for a test with no cancel of its own to
    /// check.
    fn no_cancel<F>(run: F) -> DedupReauth<F, fn(&str)>
    where
        F: Fn(&str) -> CoreResult<()> + Send + Sync,
    {
        DedupReauth::new(run, |_: &str| {})
    }

    #[test]
    fn a_successful_reauth_returns_ok() {
        let r = no_cancel(|_: &str| Ok(()));
        assert!(r.reauth("p-sales").is_ok());
    }

    #[test]
    fn a_failed_reauth_returns_its_code() {
        let r = no_cancel(|_: &str| {
            Err(CoreError::Unavailable {
                component: "test",
                reason: "login_cancelled",
            })
        });
        assert_eq!(r.reauth("p-sales").unwrap_err().code(), "login_cancelled");
    }

    #[test]
    fn concurrent_triggers_for_one_profile_run_the_flow_only_once() {
        let runs = Arc::new(AtomicUsize::new(0));
        let runs2 = Arc::clone(&runs);
        let r = Arc::new(no_cancel(move |_: &str| {
            runs2.fetch_add(1, Ordering::SeqCst);
            // Long enough that the second caller below is certain to arrive, and register as a
            // waiter, while this one is still the registered runner.
            thread::sleep(Duration::from_millis(200));
            Ok(())
        }));
        let r2 = Arc::clone(&r);
        let h1 = thread::spawn(move || r2.reauth("p-sales"));
        // The first caller registers as the runner (and so is in `inflight`) strictly before it
        // increments `runs`, so this proves the second caller will find that registration.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while runs.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "the first caller never started"
        );
        let r3 = Arc::clone(&r);
        let h2 = thread::spawn(move || r3.reauth("p-sales"));
        assert!(h1.join().unwrap().is_ok());
        assert!(h2.join().unwrap().is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn two_different_profiles_run_independently() {
        let runs = Arc::new(AtomicUsize::new(0));
        let runs2 = Arc::clone(&runs);
        let r = no_cancel(move |_: &str| {
            runs2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(r.reauth("p-sales").is_ok());
        assert!(r.reauth("p-fin").is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cancel_calls_the_cancel_closure_with_the_profile() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = Arc::clone(&seen);
        let r = DedupReauth::new(
            |_: &str| Ok(()),
            move |p: &str| seen2.lock().unwrap().push(p.to_owned()),
        );
        r.cancel("p-sales");
        r.cancel("p-fin");
        assert_eq!(*seen.lock().unwrap(), vec!["p-sales", "p-fin"]);
    }

    #[test]
    fn a_panic_in_the_flow_fails_the_slot_instead_of_blocking_its_waiters_forever() {
        let started = Arc::new(AtomicUsize::new(0));
        let started2 = Arc::clone(&started);
        let r = Arc::new(no_cancel(move |_: &str| {
            started2.fetch_add(1, Ordering::SeqCst);
            // Long enough that the waiter below is certain to register before this panics.
            thread::sleep(Duration::from_millis(200));
            panic!("the flow blew up");
        }));
        let r2 = Arc::clone(&r);
        let runner = thread::spawn(move || r2.reauth("p-sales"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while started.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "the runner never started"
        );
        // Registered while the runner above is still in flight: a waiter, not a second runner.
        // It must not block forever when that runner panics instead of returning.
        let waiter = r.reauth("p-sales");
        assert!(runner.join().unwrap().is_err());
        assert!(waiter.is_err());
        // The profile is not left stuck "in flight": a later call runs the flow again.
        assert!(r.reauth("p-sales").is_err());
    }
}
