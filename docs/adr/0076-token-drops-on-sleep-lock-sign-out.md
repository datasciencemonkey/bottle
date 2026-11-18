# ADR 0076: R6 drops tokens on the OS sleep and screen-lock events, and per profile on sign-out

- Status: accepted
- Issue: #465 (W-04b, W-18c)
- Date: 2026-11-18
- Owner: impl-465 (agent-lead-2)

## Context

W-04b says that R6 TokenBroker drops tokens on 401, sleep, lock or sign-out. W-18c says that a
sign-out overwrites the tokens of the profile with zeros. #18 (ADR 0066) built
`CliTokenBroker::drop_all` and `token_state`, but nothing called them. The issue left these
points open:

1. Which OS events count as "sleep" and "lock" on macOS and Windows.
2. Whether a sign-out drops every profile or one profile.
3. What happens to a CLI run that is in flight when a drop occurs.
4. Where the `profile_status` reply is built, because the host routing is in a contract file.

## Decisions

1. The new module `mog_core::token_events` holds the OS sources behind the
   `SessionEventSource` seam, so tests use a fake source.
   - macOS: sleep is `NSWorkspaceWillSleepNotification`. Lock is the `com.apple.screenIsLocked`
     distributed notification. Screen sleep without a lock is not a drop.
   - Windows: sleep is `PBT_APMSUSPEND` from `PowerRegisterSuspendResumeNotification`. The
     callback drops the tokens before it returns, because Windows waits for it before sleep.
     Lock is `WTS_SESSION_LOCK` to a message-only window on its own thread.
   - Wake and unlock do not fetch a token. The next call runs the CLI (W-05a).
2. `watch_token_drops(source, broker)` calls `drop_all` for each event. It holds R6 weakly, and
   a dropped `SessionWatch` stops the drops and removes the OS observers.
3. A sign-out drops one profile only (`CliTokenBroker::drop_profile`, through
   `token_events::sign_out`). The other profiles keep their tokens. Only the user's sign-out
   action (#163) calls it. Bottle never signs out by itself.
4. Each drop increments a per-profile counter. A CLI run that started before the drop gives its
   token to its caller for that one call only, and R6 does not cache it.
5. `ProfileStatusComponent` builds the `profile_status` reply from `token_state`. The reply has
   `profile` and `state` only. `expiresAt` stays unset. It never runs the CLI and never holds
   a token. The host routing and the app-lifetime watch are #486 (contract-change), on the R6
   instance that #467 builds.

## Consequences

- macOS gets the sleep notification on the main thread, and the lock notification on the main
  run loop. Both run only a mutex-guarded drop.
- The Windows watch uses one thread and one window class, `BottleTokenSessionEvents`.
- Until #486 lands, the app does not route `profile_status` and does not start the watch. Tests
  cover the full path with a fake source and a fake CLI. A local test posts a real
  `NSWorkspaceWillSleepNotification` on macOS. A Windows test sends a real
  `WM_WTSSESSION_CHANGE` to the window.
