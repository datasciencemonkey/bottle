# ADR 0010: Windows talk-key hook, key swallowing and the elevated-window tooltip

- Status: Accepted
- Date: 2026-11-16
- Issue: #7 (CAP-40a, CAP-40b, CAP-41, CAP-42)
- Amended: 2026-11-16 by #313 (CAP-40b, CAP-42, CAP-43a, CAP-43b, CAP-45): decisions 10, 11, 13 and 14

## Context

Issue #7 asks for the Windows R1 HotkeyService: `WH_KEYBOARD_LL` on a dedicated thread with a callback
under 1 ms, a new hook every 10 min and after resume, the talk key and Esc swallowed during capture, and a
tooltip when an elevated window blocks the talk key. The issue leaves open these points: which events the
hook swallows, how a reinstall avoids a stuck PTT, how the code finds an elevated window, and when the
tooltip shows. Spike SP-03 (#5) is still open.

## Decision

1. The decision logic is platform-neutral code in `crates/mog-core/src/hotkey_windows.rs`. Tests run it on
   every OS. The Win32 glue is in `hotkey_windows/win32.rs` and builds only on Windows.
2. One thread owns the hook and its message loop. The callback runs `HookCore::handle`: a fixed-size
   filter, one channel send and one latency sample. It takes no lock and does no I/O. The unbounded
   channel send can allocate a block now and then. A mask replay is posted to the loop, so `SendInput`
   never runs inside the callback.
3. The filter swallows the key-down that completes the talk key, its auto-repeats and its key-up. It
   swallows Esc while the talk key is held or hands-free capture is on. Esc in a meeting session passes.
4. The filter swallows a key-up only when it swallowed the key-down, so no key sticks in the focused app.
5. Injected events (`LLKHF_INJECTED`) pass. Bottle's own paste and the mask replay must reach the app.
6. For the Ctrl+Win talk key, a Win key that went down before the chord gets a masked key-up: a tap of
   `vkE8`, then the Win key-up, all injected. This stops the Start menu from opening. When Win goes
   down first, Windows saw the Win key-down, so a key pressed during that hold can fire a Win+key
   shortcut. The default talk key (Right-Alt) does not have this problem.
7. A 1 min timer drives `HookSupervisor`. Every 10 min it installs a new hook and then removes the old
   one, so there is no gap. If the install fails, the old hook stays.
8. After a timer reinstall, the code reads `GetAsyncKeyState` only for keys whose key-down reached
   Windows. Windows does not update the async state of a key that the hook swallowed, so the code does
   not end a PTT from that state. A lost key-up of a swallowed talk key is bounded by the 120 s hold stop.
9. On resume, an active PTT ends and all key state clears (a hold cannot last through sleep), then the
   hook is installed again. Windows can send two resume events; the second one only installs again. A key-up
   of a key swallowed before the sleep then reaches the app without its key-down; that is harmless.
10. A failure at start (hook, thread, or the 1 min timer) gives `CoreError::Unavailable { component: "R1",
    reason }`. A later failure (reinstall, resume notification, foreground hook) sends
    `HookSignal::HookFailed { reason }`. The next tick tries a failed reinstall again. If `GetMessageW`
    returns -1, the thread sends `HookFailed { reason: "message_loop_failed" }`, removes the hook and ends.
    A finished thread counts as stopped: `stop()` only joins it, and a later `start()` installs a new hook.
11. Elevated window: a foreground WinEvent hook sends the process ID to a second thread. That thread reads
    the token elevation. If Bottle is not elevated and the foreground process is, the core sends
    `TalkKeyBlocked` once per process ID per run. A process that opens but refuses the token query counts
    as elevated. A process that does not open counts as unknown and sends nothing. At start, the hook
    thread reads `GetForegroundWindow()` once and sends that process ID first, so an elevated window that
    already has the foreground gives `TalkKeyBlocked` at once (#313).
12. Signals name only the talk key, Esc or "another key". No signal, error or log names another key.
13. Talk keys on Windows (#313): D66 gives Windows only Right-Alt, Right-Ctrl and Ctrl+Win, so the Windows
    `TalkKey` has only these three. A swallowed Right-Shift blocks capitals, and a swallowed Right-Win
    breaks the Win shortcuts. `hotkeys_set` on Windows refuses `RightWin` and `RightShift` as `invalid` and
    keeps the old talk key. The conflict check flags a stored one as `invalid`, with the next free step of
    the ladder as the fix. A shared setting of Right-Command or Right-Shift maps to Right-Alt on Windows.
14. Session unlock (CAP-45, #313): a message-only window on the hook thread registers with
    `WTSRegisterSessionNotification`. On `WTS_SESSION_UNLOCK`, the state of every swallowed key clears,
    because its async state is not real. The passed keys get their `GetAsyncKeyState` value again. An
    active PTT ends. A failed registration sends `HookFailed { reason: "session_notification_failed" }`
    and the hook keeps running.

## Consequences

- The tooltip shows when the elevated window takes the foreground, before the first press. A press in that
  window cannot reach the hook, so this is the earliest point that the core can see the failure.
- The MOG tooltip needs a core-to-brain event. That is a contract change: #294. Until it lands, the signal
  stays on the in-process channel.
- TODO(#5): SP-03 must confirm on a Windows device that UIPI blocks the hook under an elevated window, and
  measure the real callback p99 and the 8 h soak (AT-30). The device steps are in #296 (needs:windows).
- #6 merged `hotkey_core.rs`. The shared `hotkey_core::TalkKey` converts to the Windows `TalkKey` (which adds
  the Ctrl+Win chord), and `forward_to_engine` feeds `TalkDown`/`TalkUp` to the shared `HotkeyEngine`, so
  both platforms give the same `hotkey.ptt` phases. The tap watchdog of #6 is macOS-specific; Windows uses
  `HookSupervisor`. TODO(#14): the capture lifecycle creates the service and connects it to the engine.
