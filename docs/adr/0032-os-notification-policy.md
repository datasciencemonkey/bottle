# ADR 0032: OS notification policy in the brain

- Status: Accepted
- Date: 2026-11-16
- Issue: #95 (UX-26a, UX-26b, UX-26c; ADR 0004 thin views)

## Context

The app sends an OS notification only when the card is hidden and the OS Focus mode permits it. It sends 1 notification or less for each MOG in each 10 s. It merges notifications of different MOGs that occur in the same 5 s. A chip never notifies.

## Decisions

1. **The brain owns the policy (ARC-2).** `src/brain/notification-service.ts` takes a `MonotonicClock`, a `Scheduler` and a `NotificationHost`. The tests use the sim clock.
2. **The first report opens a 5 s window.** Reports of other MOGs in the window join it. A second report of the same MOG in the window adds nothing. At the end of the window the service sends one notification: the text of the answer for one MOG, "N MOGs have new answers" for more.
3. **The checks run at the end of the window.** The service reads the Focus mode and the card state of each MOG then, so a card that opens during the window cancels its notification. A MOG with a visible card leaves the merged notification.
4. **The 10 s limit starts at the send time.** A report inside 10 s of the last notification of that MOG is dropped. The answer stays in History.
5. **A failed host call sends nothing.** An unknown card state or Focus state is not a reason to disturb the user.

## Consequences

- A notification comes up to 5 s after the answer. This is the cost of the merge (UX-26c).
- The host has no IPC command for `cardVisible`, `focusAllows` or `notify` yet. The contract (`src/shared/ipc.ts`, `mog_core::types`, `src-tauri/src/ipc.rs`) is not changed here. The wiring in `main.ts` waits for that change (`TODO(#95)`).
