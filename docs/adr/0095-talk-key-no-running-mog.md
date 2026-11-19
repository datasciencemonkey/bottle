# ADR 0095: The talk key with no running MOG starts the last-used MOG

- Status: Accepted
- Date: 2026-11-18
- Issues: #562 (MB-06a), after #73 (ADR 0058) and #14
- Owner: impl-562 (agent-lead-2)

## Context

The host latches the talk-key target at key-down (`PttTarget`, ADR 0058). Until now, with no open MOG window, the press sent nothing to the brain. A MOG window opens only on a hub Start, so a talk-key press after a new launch did nothing that the user could see. #562 says that the press must start the last-used MOG and capture into it, or the hub must show "Create a MOG to start".

The host does not know the MOG configs, the run state of T1 or the hub list. The brain knows all of them.

## Decisions

1. **The host sends the press with an empty `mogId`.** With no target at key-down, `PttForwarder` latches `""`. The `down`, `up`, `cancel` and `double_tap` of that press all go as `hotkey:ptt { mogId: "", phase }`. This is the same payload shape as section 4.5. No contract file changes. RTR-12a uses `""` for "no MOG" in the same way.
2. **Only `talk-no-mog.ts` reads an empty `mogId`.** `asPttEvent` and the talk-target wiring drop it, so the capture lifecycle, the ask gate and the target never see a MOG with the ID `""`.
3. **The key-down starts the MOG through T1.** `MogManager.start` opens the `mog-<id>` window with `window_mog_open`, as the hub Start does. Then the brain sends each phase of the press on the bus again, with the ID of the started MOG. The T12 consent gate and the capture lease rules apply as for any press. An `up`, `cancel` or `double_tap` alone never starts a MOG.
4. **The last-used MOG.** It is `TalkTarget.lastUsed()`: the target now, or the last target before every MOG closed. Bottle does not save it on disk. In a new run, or when that MOG was deleted, it is the first MOG of the hub list (sorted by name).
5. **The hub shows the result.** With no MOG, the hub list gets the info notice "Create a MOG to start". A refused start shows the T1 error text. After a start, the brain sends the hub list again, so the MOG shows as running.

## Consequences

- A talk-key press is never dropped with no visible result while the hub is open.
- When the hub window is not open, the "Create a MOG to start" notice is lost. The hub asks for the list again when it opens. #568 (a contract change) tracks a way to show it with no hub window.
- T1 keeps its run state in brain memory only (#419). The empty ID proves that no MOG window is open, so when T1 says the picked MOG is running, the brain also calls `window_mog_open` (idempotent) for it. So a capture never starts for a MOG with no window.
- The capture of the first press starts after the MOG window opens, so it starts later than a press to a running MOG.
- If a later change saves the last-used MOG (for example in the app settings), only `lastUsed` changes.
