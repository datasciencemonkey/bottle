# Notes on the parallel Kimi baseline (requirements_kimi.html)

The full file is not reachable: it lives on another session's host (conversation 4145830775601994, runner offline). These notes come from that session's transcript, read on 2026-11-14.

## Shape
- "Motherbricks End-to-end MOG requirements", v1.0, director-approved baseline. It has 180 requirements and 22 acceptance tests (AT-01..AT-22), and every requirement links to a test.
- It has five inline SVGs: a one-page overview, 01-system, 02-voice-sequence, 03-state and 04-trust. It cites 23 sources.
- It is "STE100-style" and uses "shall" (for example "Motherbricks shall require a trusted native confirmation before hands-free dictation starts.").
- Requirement prefixes: MB-, CAP-, RTE-, WS-, SEC-, G-gates (G5 = Lakebase gate).

## Strong ideas worth adopting
- Native **Audio Broker / CaptureSession**: hotkey leases, one owner per source, no generic webview capture command, separate PTT and meeting buffers (CAP-012), and a 120-second PTT safety stop.
- The microphone-release signal is modelled as Active / Released / Unknown. Unknown never stops capture, because the OS gives no universal signal.
- A persistent native recording indicator that fails closed (CAP-007/008). Trusted native confirmation before hands-free or meeting capture (CAP-005/006). A calendar event is not consent.
- **Route and Context Gate**: outbound context to Genie needs approval bound to ONE immutable payload, target, conversation and attempt, and any change invalidates it. Context carries its source, timestamp, truncation state and a byte limit (RTE-008/009, MVP). Audit metadata comes in the MVP (SEC-017).
- **Operation states** NOT_SENT / SENDING / ACKNOWLEDGED / UNCERTAIN. An uncertain submission never causes an automatic duplicate question. A **Recovery Manager** reconciles after a restart, and stale callbacks are discarded.
- Credential Broker: tokens stay in native volatile memory. Tokens are bound to the workspace origin, with a separate origin-bound lease for Databricks Apps (SEC-016). Authenticated cross-origin redirects are rejected.
- `?o=<workspace-id>` routing on the Genie One MCP URL. No `CANCELLED` terminal poll state (only "cancellation requested"). Each Genie agent-mode operation's launch status is checked separately, not labelled Beta as a whole.
- Memory: local encrypted recall uses no network. Saving, syncing to Lakebase and sending context outbound are three separate approvals. It has an outbox, tombstones and retention.
- Physical-device and live-workspace tests are release gates, not reasons to block the requirements.

## Deviations from the user's vision (collisions to resolve)
- K1 Naming: it uses "Motherbricks" and asserts that "Mother Databricks" and "Mother Bricks" never appear. The user named the project "Mother Databricks" and the app "Mother Bricks".
- K2 Lakebase is OPTIONAL in the MVP (local-first). The user wants MOG memory in Lakebase for fast lookup.
- K3 "Explicit routes": the user reviews the route and target before a question goes out. The user's vision is an always-on MOG that shares questions and context with Genie automatically.
- K4 Meeting / always-on capture is v1, not MVP.
- K5 Only 4 detailed diagrams. It has no dedicated Genie ask/poll sequence, workspace-hopping, memory-flow or meeting-stop-policy diagram.
- K6 It uses "shall", where our STE100 rule set uses "must".
