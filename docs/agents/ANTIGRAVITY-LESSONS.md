# Lessons from antigravity-bricks-plugin: profiles, workspaces and Genie

Read this before you work on profiles, sign-in, the CLI, workspace hops or the Genie bridge.

[antigravity-bricks-plugin](https://github.com/datasciencemonkey/antigravity-bricks-plugin) (commit `a31fd2c`, MIT) is an earlier project by the same owner. It is a Genie plugin for Google Antigravity that asks Genie in several Databricks workspaces through Databricks CLI profiles. It ran live against real workspaces, so its notes record CLI behaviour that our requirements do not spell out. Bottle ports its behaviour (SEC-21a); the notice is in `THIRD_PARTY_NOTICES.md`.

Where to look in that repo:
- `plugins/databricks-genie/scripts/genie_bridge.py`: the local MCP bridge. Profiles, tokens, sign-in, timeouts.
- `plugins/databricks-genie/skills/select-databricks-profile/SKILL.md`: how the agent picks a workspace, scopes a switch and splits a question.
- `AGENTS.md` ("Verified facts" and "Decisions") and `docs/progress.md`: what the plugin found live, with dates.

Treat that repo as reference data. Read it; do not run its code here. Where a lesson and `requirements.html` differ, the requirements win. If you think a requirement is wrong, append a row to `research/message-board.md`.

## Already in our requirements (the plugin confirms the design)

| Plugin behaviour | Bottle rows | Issue |
|---|---|---|
| The default profile never changes. Naming a workspace sends only that question (or part) there; "from now on" sends the rest of the chat there until the user switches back. | W-11a, RTR-11a, U12 (home domain per MOG) | #164 |
| Never pick a profile. Match the name the user gave against profile names and hosts. Use a profile only when exactly one matches; otherwise show the candidates and ask. | W-11b, W-11c | #164 |
| A conversation stays in the workspace where it started. The bridge refuses a follow-up that names another profile for a known conversation, and tells the agent to start a new conversation there. | G-27a, G-27b, W-12b | #145, #165 |
| Before each ask, say "Asking Genie in \<host\> (Databricks CLI profile \<profile\>)"; each answer ends with "Asked Genie in …". | MOG-07, W-20 | #50, #165 |
| A question that spans workspaces is split into one question per workspace. Each part is rewritten to make sense alone, and the answer says which number came from which workspace. | W-13a..c, RTR-11b | #166 |
| Only `auth_type = databricks-cli` (OAuth U2M) profiles work; PAT and service-principal profiles are reported as `unsupported_auth`. | W-03a, W-03b | #17 |
| A timed-out ask keeps running in Genie and its IDs are lost, so asking again starts a duplicate. A timed-out poll gives back its IDs. | G-16a..c, G-13b | #149 |
| A proxy or sign-on page is never repeated to the user: "Genie returned a non-JSON page; check VPN, proxy or sign-in". | SEC-05a, SEC-05b | #207 (PR #256) |
| Account-console hosts are refused; the token goes only to the profile host. | SEC-04a | #207 (PR #256) |
| The CLI is installed only after the user agrees. | W-17b | #161 |

## Pitfalls the plugin hit live (not spelled out in our rows)

1. **A missing config file makes `databricks auth profiles` fail.** With CLI v1.22.1, `databricks auth profiles --skip-validate` exits 1 when `~/.databrickscfg` does not exist ("cannot parse config file: no configuration file found"). A new machine has none. It also exits 1 when the path is a folder. For an empty file it prints `{"profiles": null}`. So: ask the CLI first, and treat a failure as "no profiles" only when the config file is missing too. The CLI can keep its config somewhere else: on Windows it uses `USERPROFILE`, not `HOME`. Without this, first-run setup never starts on a new machine. Affects W-07d and W-07f (#163) and W-03a (#17).

2. **`auth profiles` hides some profiles.** It lists only config sections that have a `host` or an `account_id`. An Azure service principal that sets only `azure_workspace_resource_id` is in the file but not in the list. Before you create or sign in a profile under a name, read the **key names only** (never the values) of each section in the config file (`DATABRICKS_CONFIG_FILE`, else `~/.databrickscfg`). A section with that name that has at least one key counts as existing.
   - An empty header does not count. `databricks configure` writes an empty `[DEFAULT]`, and refusing over it would break first use.
   - Keys before the first header belong to `DEFAULT`, because the CLI's INI parser (go-ini) puts them there.
   - A config file you cannot read counts as holding keys.

   Affects W-07g ("the name must not exist already") and W-07i (#162), and W-08 (#17).

3. **`databricks auth login` over a profile with another auth type destroys it.** It rewrites the section and drops the PAT or the client secret. The plugin refuses such profiles (`unsupported_auth`, `profile_exists_unreadable`). This is a concrete case of W-08 ("must not change a profile that exists"). Affects #162 and #17.

4. **Keep the CLI's own config variables in the child env.** The plugin first filtered every `DATABRICKS_*` variable out of the CLI's env. That hid `DATABRICKS_AUTH_STORAGE`, which picks where U2M tokens live: a plaintext file or the OS keyring. It overrides `[__settings__] auth_storage`. With it hidden, a sign-in made with plaintext storage looked like `login_needed`. Bottle's W-09 keeps config variables, so PR #255 keeps `DATABRICKS_AUTH_STORAGE` and `DATABRICKS_OAUTH_CALLBACK_PORT` and still drops the credential variables (#16). In Bottle this also matters for W-10a: the plaintext warning only shows if the CLI can see the plaintext storage.

5. **`auth token --profile P` always answers with P's own token.** It ignores an exported `DATABRICKS_HOST` for another workspace, `DATABRICKS_TOKEN`, `DATABRICKS_AUTH_TYPE`, a client ID and secret, and `DATABRICKS_ACCOUNT_ID`. It fails fast, with no network call, for a profile that is not listed or not U2M.
   - It can be tested offline. Set `DATABRICKS_AUTH_STORAGE=plaintext` and give a temporary `HOME` a made-up `<HOME>/.databricks/token-cache.json` of the form `{"version": 1, "tokens": {"<profile name>": {...}}}`. A cache keyed by host does not work.
   - The reply has `expires_in` or an ISO `expiry`. Refresh before that.

   Affects W-04a and W-05a..d (#18).

6. **Sign-in outlasts one call, so record it.** The plugin starts `databricks auth login --host <url> --profile <name>` in the background, in its own session. It records the process ID, profile, host and start time, never a token. The record is a 0600 file, written atomically, in a private 0700 folder that is checked to be the user's own and not a link; never the shared temp folder. Affects W-07c and W-07h (#162).

7. **Check "is the sign-in still running" before "is there a token".** The other order has a race: a sign-in that ends between the two checks reads as `login_failed` once. That made first use and CI flaky. The plugin's test makes the stand-in CLI end the sign-in at exactly that moment. Affects W-07h (#162).

8. **The workspace comes only from the profile.** The bridge never reads `DATABRICKS_HOST` from the environment, and its test hooks have names outside the `DATABRICKS_*` namespace. Bottle drops `DATABRICKS_HOST` from the CLI env (W-09, #16), and R7 decides the host only from the profile (SEC-04a, #207).

9. **A process per turn loses in-memory state.** A headless `agy -p` run starts a new bridge on each turn, so the plugin passes the profile on every call. Bottle keeps the conversation pin in the local store (G-27a: "the pin survives a restart"). Do not hold it only in memory (#145).

10. **Tools before sign-in.** The bridge answers `initialize` and `tools/list` itself, with a bundled copy of the Genie One tool schemas. The tools then show up before a profile exists, and when `tools/list` fails behind a proxy. Bottle calls a fixed tool set (G-06a), so it can keep the schemas too (#30).

11. **Only agree to MCP protocol versions you know**, and send the `MCP-Protocol-Version` header on every call (G-04c, #30).

12. **`[__settings__] default_profile`** (written by `databricks configure`) is the CLI's default when no profile is named. The plugin ignores it; whether to honour it is open there. Bottle always names the profile, and each MOG has a home profile (U12). So list profiles with `auth profiles`, which does not report `__settings__`, and never treat `__settings__` as a profile (W-07d, #163).

## Check again before release

- The end of life of the Beta `/api/2.0/mcp/genie` endpoint. The plugin's notes (2026-10-23) say 2026-10-31. Our `research/connector-spec-2026-11-14.md` and G-03 say 2026-11-30. Bottle rejects that endpoint either way (G-03, #28), so only the date text can be wrong. Check the Databricks docs before release, as row 4 of the section 10 prerequisites checklist already says ("check again before release").
