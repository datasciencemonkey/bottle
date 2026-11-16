# ADR 0010: Windows talk-key hook, key swallowing and the elevated-window tooltip

- Status: Accepted
- Date: 2026-11-16
- Issue: #7 (CAP-40a, CAP-40b, CAP-41, CAP-42)

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
   filter, one channel send and one latency sample. It takes no lock and does no I/O.
3. The filter swallows the key-down that completes the talk key, its auto-repeats and its key-up. It
   swallows Esc while the talk key is held or hands-free capture is on. Esc in a meeting session passes.
4. The filter swallows a key-up only when it swallowed the key-down, so no key sticks in the focused app.
5. Injected events (`LLKHF_INJECTED`) pass. Bottle's own paste and the mask replay must reach the app.
6. For the Ctrl+Win talk key, a Win key that went down before the chord gets a masked key-up: a tap of
   `vkE8`, then the Win key-up, all injected. This stops the Start menu from opening.
7. A 1 min timer drives `HookSupervisor`. It removes the old hook and installs a new one every 10 min. A
   suspend/resume notification does the same on resume. After each reinstall, the code reads the physical
   key state with `GetAsyncKeyState`. A talk key that is no longer down ends the PTT (a lost key-up).
8. An install failure gives `CoreError::Unavailable { component: "R1", reason }`. The next tick tries again.
9. Elevated window: a foreground WinEvent hook sends the process ID to a second thread. That thread reads
   the token elevation. If Bottle is not elevated and the foreground process is, the core sends
   `TalkKeyBlocked` once per process ID per run. An unknown elevation (access denied) sends nothing.
10. Signals name only the talk key, Esc or "another key". No signal, error or log names another key.

## Consequences

- The tooltip shows when the elevated window takes the foreground, before the first press. A press in that
  window cannot reach the hook, so this is the earliest point that the core can see the failure.
- The MOG tooltip needs a core-to-brain event. That is a contract change: #294. Until it lands, the signal
  stays on the in-process channel.
- TODO(#5): SP-03 must confirm on a Windows device that UIPI blocks the hook under an elevated window, and
  measure the real callback p99 and the 8 h soak (AT-30). The device steps are in #296 (needs:windows).
- When #6 merges its shared PTT state machine (`hotkey_core.rs`), the Windows signals map to its types.
