//! The home-first "sign in, then retry" rule (ADR 0122 amendment, owner decision 2026-11-24).
//!
//! The owner's rule (verbatim): "In the absence of a sign-in, or if the sign-in had expired,
//! refresh the sign-in. The user is always going to be watching it, so it doesn't matter. You
//! can just open up the browser to refresh the sign-in using the U2M OAuth."
//!
//! A `signed_out` `ask_genie` call never widens the home-first rule: home has not said it lacks
//! data, its token has merely expired. Instead the orchestrator host starts home's browser
//! sign-in (the same U2M OAuth flow as `profile_reauth`, "Sign in…" in Settings → Workspace) and
//! retries the call once. [`ReauthService`] is that flow, as the orchestrator sees it: no new
//! OAuth implementation, no token in Rust's hands beyond what `profile_reauth` already keeps.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use crate::error::{CoreError, CoreResult};

/// Starts a profile's browser sign-in and blocks until it ends, fails, is cancelled or times
/// out, with whatever bound the underlying flow already has. Implementations must dedupe: a
/// second call for a profile that is already signing in must wait for that attempt's result
/// instead of starting a second browser window ([`DedupReauth`] does this for any inner flow).
pub trait ReauthService: Send + Sync {
    fn reauth(&self, profile: &str) -> CoreResult<()>;
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
/// browser window ever opens for that profile at once.
pub struct DedupReauth<F> {
    run: F,
    inflight: Mutex<HashMap<String, Arc<Slot>>>,
}

impl<F> DedupReauth<F>
where
    F: Fn(&str) -> CoreResult<()> + Send + Sync,
{
    pub fn new(run: F) -> Self {
        Self {
            run,
            inflight: Mutex::new(HashMap::new()),
        }
    }
}

impl<F> ReauthService for DedupReauth<F>
where
    F: Fn(&str) -> CoreResult<()> + Send + Sync,
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
            let result = (self.run)(profile).map_err(|e| e.code());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_successful_reauth_returns_ok() {
        let r = DedupReauth::new(|_: &str| Ok(()));
        assert!(r.reauth("p-sales").is_ok());
    }

    #[test]
    fn a_failed_reauth_returns_its_code() {
        let r = DedupReauth::new(|_: &str| {
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
        let r = Arc::new(DedupReauth::new(move |_: &str| {
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
        let r = DedupReauth::new(move |_: &str| {
            runs2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(r.reauth("p-sales").is_ok());
        assert!(r.reauth("p-fin").is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
