# genie 0.5.0 app-server transcripts (sanitized)

Source: `bottle-tools/genie-code-spike/runs2/as-eph.jsonl` and `as-onrequest.jsonl` (REPORT2.md).
Paths, host names, the machine name and the installation id are replaced. The model text is
the fake MCP server's table data. There is no token and no profile name.

- `appserver-ephemeral-turn.jsonl`: one ephemeral thread with `approvalPolicy:"never"` and one
  turn (initialize, thread/start, turn/start, items, token usage, turn/completed). The CI test
  `session::tests::fixture_*` replays it.
- `appserver-approval-request.jsonl`: a server-to-client approval request (`on-request` policy).
  Bottle must refuse it and end the session (AC-2a.23).

`GENIE_PINNED` must equal the suffix of this folder (`genie-<version>`). A pin bump needs new
fixtures and a new run of `tools/genie-lockdown` first.
