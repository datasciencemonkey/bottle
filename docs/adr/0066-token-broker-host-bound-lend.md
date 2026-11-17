# ADR 0066: R6 TokenBroker lends a token only for its bound host, and reads the org ID through R7

- Status: accepted
- Issue: #18 (W-01b, W-04a, W-04b, W-05a, W-05b, W-05c, W-05d, W-06a, W-06b)
- Date: 2026-11-17
- Owner: impl-18 (agent-lead)

## Context

R6 TokenBroker is the only holder of tokens (ARC-6). The requirements say that a token goes
only to the exact host of its profile (4.6 rule 2), that R6 refreshes at expiry - 60 s with one
CLI call per profile (W-05a), and that R6 reads `workspace_id` from the `x-databricks-org-id`
header of `GET /api/2.0/preview/scim/v2/Me` (W-06a). The issue left these points open:

1. How R6 gets the `auth token` output when R5 `run_json` refuses that command for every caller.
2. Where the host check lives. Before #18, R7 and the model proxy checked the host, but
   `TokenBroker::with_token` took only a profile, so R6 could not refuse a wrong host.
3. How R6 makes the SCIM call. R7 sends HTTP and holds R6, and its `request` returns the body only.
4. What R6 does when the CLI reply has no expiry.

## Decisions

1. `CliRunner::run_auth_token(profile)` is the only path for `auth token --profile <p> --output
   json` (`cli_runner::auth_token_args`). It returns the raw stdout to R6 as a `SecretStdout`
   (not `Clone`, redacted `Debug`, zeroed on wipe and on drop). R6 parses it with one copy of the
   token into a `SecretToken` and wipes the stdout on success and on failure. A source
   scan test allows only `token_broker.rs` and `cli_runner.rs` to name `run_auth_token`. The real
   child process is TODO(#16); the trait default is `NotImplemented`.
2. `TokenBroker::with_token(profile, host, f)` takes the exact host of the request. R6 binds each
   token to the lowercase host of its profile URL when it gets the token. It refuses any other
   host with `token_host_mismatch` and does not call `f`. R7 passes the host of the URL it sends to,
   and the model proxy passes its gateway host. This is defence in depth: the R7 egress policy
   still runs first.
3. R6 defines the `ScimMe` seam. `ReqwestHttpTransport` implements it through its own request
   pipeline (host policy, token from R6, 401 retry), and returns only the header. R6 holds the
   seam as a `Weak` (`bind_scim`), so R6 and R7 do not form an `Arc` cycle. R6 caches the ID per
   profile for the process lifetime. A failure is not cached. A dropped token does not drop the ID.
4. If the CLI reply has neither `expires_in` nor `expiry`, R6 uses the token and refreshes after
   300 s (`DEFAULT_LIFETIME`), as the reference plugin does. A past or unreadable expiry is
   `token_reply_invalid`.
5. Leases: `with_token` returns a `TokenLease` that names the token it lent, and
   `invalidate(profile, lease)` drops the token only if it is still that token. So when N MOGs get
   a 401 for one token, the late 401s do not drop the new token, and the CLI runs once (W-05a,
   W-05c). Sign-out, sleep and lock use `drop_all`.
6. Single-flight: one CLI run per profile at a time. Callers that arrive during the run wait on a
   condition variable and share its result, including a failure (`login_required`), so N MOGs
   cause one CLI call. Other profiles are not blocked.

## Consequences

- Every `TokenBroker` implementation and caller passes the host and keeps the lease for a 401.
  Test doubles ignore both.
- `profile_status` IPC wiring to `CliTokenBroker::token_state` is a follow-up (the handler is in a
  contract file). The OS sleep, lock and sign-out events must call `drop_all` (TODO(#163)).
- The R7 401 rule did not change: invalidate once, retry once, then `auth_expired`. The UI
  shows the Table W-ERR "Login required" text for it.
