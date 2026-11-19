# ADR 0100: "Connect a workspace" signs in with one blocking `profile_login` and a cancel row

- Status: accepted
- Issue: #162 (W-07a, W-07b, W-07c, W-07f, W-07g, W-07h, W-07i, W-15)
- Date: 2026-11-19
- Owner: impl-162 (contract owner: agent-lead-5)

## Context

The form runs `databricks auth login` in the background, shows Cancel and "Open the browser
again", and checks the new profile. 4.5 has `profile_login {workspaceUrl, profile, clientId?}
-> {loginId}` only. It has no cancel row and no result event that reaches the hub. ARC-2 lets
views call only `view_*` commands, so the form logic must run in the brain.

## Decisions

1. `profile_login` blocks until `auth login` and the W-06a check end (contract owner decision).
   It returns `{loginId}` on success. Otherwise it rejects with one fixed code: `host_refused`,
   `profile_exists`, `invalid_args`, `login_cancelled`, `timeout`, `cli_failed`, `auth_expired`,
   `no_grant`, `non_json_reply`, `offline` or `host_failed`. It runs on a worker thread.
2. One new row, `profile_login_cancel {profile} -> {}` (brain and hub). It kills the running
   child for that profile. An unknown or finished profile gives `{}`. No status row, no polling.
3. "Open the browser again" is Cancel, then the same `profile_login` again. The CLI opens a new
   browser page. Bottle does not read the sign-in URL from the CLI output.
4. The host checks the URL again (`mog_core::profile_login::workspace_host`, 4.6 rule 2 through
   `is_workspace_host`), checks the name against the CLI config file, and passes only flag pairs
   to R5. R5 `run_login` refuses any command but `auth login` and wipes the child output.
5. The W-06a check after the sign-in reads the workspace ID through R6 (`TokenBroker`). The
   Genie One probe has no host seam yet, so it is not part of the check (a known MVP gap).
6. The brain (`src/brain/connect-wiring.ts`) owns the form rules and sends the model to the hub
   on `hub:connect_workspace`. The hub view only shows it and sends `view_emit` clicks.
7. At first run (W-07f) the brain opens the URL step when the hub is ready and `profiles_list`
   has no supported profile (`profiles_list`, routed by #555). If the call fails, the form
   opens only from its button.

## Consequences

- A sign-in holds the R5 read lock for up to 5 minutes. Other CLI children still run, because
  they take the read lock too. Only a new launch check waits.
- The 403 IP access list case of Table W-ERR shows as `non_json_reply` when the reply is a page.
  A JSON 403 shows as `no_grant`. R7 does not tell the two JSON cases apart yet.
