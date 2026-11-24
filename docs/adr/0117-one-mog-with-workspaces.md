# ADR 0117: One MOG with workspaces

- Status: accepted (slice 1, S1-A: the Rust part)
- Date: 2026-11-23
- Owner: one-mog (the owner approved the work on 2026-11-23)
- Changes: ADR 0047 (running cap), ADR 0058 (talk-key target), ADR 0103 (recording profile)

## Context

Until now Bottle could run up to 8 MOGs. Each MOG had its own home profile (`binding.home`), name and colour,
and the talk key went to the last MOG that the user used (ADR 0047, ADR 0058). The owner wants one MOG. Each
connected Databricks CLI profile is a workspace of that MOG. The user switches each workspace on or off in
Settings → Workspace. The home workspace is always on. A new connection starts off.

The old MOG configs hold the name and colour, and the local store holds the history and the memory folders by
`mog_id` (`/memories/<mog-slug>/`). A schema change of `MogConfig` or a move of data is not acceptable risk for
this change.

## Decisions

### One MOG, and the workspace is a CLI profile

- `MAX_ACTIVE_MOGS` is 1 (`src-tauri/src/windows.rs`). A second `window_mog_open` answers `mog_cap_reached`
  (AC-1.9). The R11 ledger keeps its cap argument, so the ledger tests can still use a larger cap.
- A workspace is one CLI profile (one profile is one workspace, AGENTS.md §1). Bottle lists every profile of the
  T7 list. A profile that T7 does not support (PAT, service principal, plaintext token storage) is
  `unsupported`. A profile in the store that is not in the CLI config is `missing`.

### The store: `workspaces.json`, not a `MogConfig` change

- The new file is `<app config dir>/workspaces.json` (0600, atomic write, `schemaVersion` 1). It holds the home
  profile, the lamp config id, one entry per profile (`mogIds`, `available`) and the agent settings
  (`crossWorkspaceData` D1, `genieCallBudget` D2, `endpoint` D9). It holds no secret.
- The old config files do not change. `mogIds[0]` of a workspace is the config that owns its name and colour.
  History and memory stay under their old `mog_id`, so no data moves (AC-1.2, AC-1.4).
- There is no `MogConfig` schema change, no C9 schema change and no migration of the SQLCipher store.
- The code is `mog_core::workspaces::WorkspaceStore`. A bad file (not JSON, wrong schema version, an unknown key, a
  budget other than 5, 10 or 20, a bad endpoint name) is an error (`workspace_storage_failed`). Bottle never
  deletes or rewrites it. A refused write changes nothing on disk.

### The migration (first run, file missing)

1. Read every readable MOG config. Group the configs by `binding.home`, with the ids sorted.
2. Each group becomes one available workspace (AC-1.1, AC-1.3).
3. `lampMogId` is the lowest config id.
4. `home` is set only if exactly one distinct profile exists and every config file was readable. Otherwise
   `home` is `null`: an unreadable config may hide another home.
5. The old configs are not modified. The agent settings start at D1 off, D2 10 and the endpoint
   `databricks-claude-sonnet-5-5`.

The migration runs once. A config that a later install adds does not join an existing file: the brain adds its
entry when the user switches the workspace on (`workspace_set_available` with a `mogId`).

### No silent home pick (FLAG 5)

Bottle never picks a home for the user. With more than one old home, `home` stays `null` until the user picks. The
brain then shows the pick card on ⌥G and the hub shows the "Choose your home workspace" dialog (the suggestion is
only a preselection). The first Connect with no home makes that new profile the home (`workspace_make_home`, the
user's own connect). `workspace_make_home` needs a supported profile and sets it available.

### The allow-list in R8 (FLAG 7)

- `McpCallComponent` takes an optional `WorkspaceGate`. For tool `genie_ask` it asks `allows(profile)` before it
  builds the call, so R8 sends nothing for a refused profile. The check is on the component, so it holds on every
  brain path (with or without an `agentCall` marker, any meta or timeout).
- The agent's Genie proxy (`GenieProxyDeps::with_gate`) asks the same gate for `genie_ask`, before the in-flight
  ledger, and gives the user's approval back. In the app, the agent host takes the gate when slice 2 wires it.
- `allows` is true for the home profile and for a profile that is switched on. A profile that is in the CLI config
  but has no entry (a new connection) is off. With no home, only the available profiles are allowed.
- It fails closed: a store that cannot be read allows nothing, and a host with no store allows nothing.
- Polls, cancels and query results of calls that are in flight still run (they are other tools). `tools/list` is
  not gated. In slice 1 polls and query results are not gated at all. This is an accepted risk for now: before
  the agent slices give the model these tools, they will be gated by recorded (profile, conversation) pairs.
- The refusal code is `workspace_not_available` (AC-1.7).

### Sign-in state

`workspace_list` merges the T7 profiles with the store. The state comes from R6 `token_state` (a cache that reads
`unknown` until the first token use). With `refresh`, the store probes each supported `unknown` profile with
`with_token` on an empty closure (so no token leaves R6). A counting semaphore holds the probes to 4 at a time. A
slot comes back only when its probe thread returns, even late. One `workspace_list` call has one overall deadline
of 10 s. A row whose probe is not done then stays `unknown`. A token failure that means "log in again" is
`expired`. Any other failure, or a timeout, leaves the row `unknown`. Until R6 separates network errors from
sign-in errors, `expired` means "the CLI could not give a token".

### Host order

`workspaces_host::install` runs after `mog_config_host::install` and `profiles_host::install`, and before
`mcp_host::install`, which takes the gate. R6 is bound to the store after R8 is built.

## Changes to earlier ADRs

- **ADR 0047 (running cap).** The cap of 8 running MOGs and the "8 MOGs are running" text no longer apply. The
  cap is 1 and the hub has no MOG create, duplicate or delete (slice 1, brain and hub PRs). The config store and
  its commands stay.
- **ADR 0058 (talk-key target).** The target is always the one lamp. The target toast and the target marker go
  away (brain PR).
- **ADR 0103 (call recording).** ⌥R records with the home profile. The `profiles_differ` check goes away (brain PR).
  A recording with no home gets `no_profile`.

## Open items

- **Re-sign-in (`profile_reauth`).** Built. It runs `databricks auth login --profile <p> --host <host>` (CLI 1.26)
  for a profile with `auth_type = databricks-cli` only. Checked with a fake config and a local mock OAuth server:
  a U2M profile keeps all its other keys (`warehouse_id`, `scopes`, `client_id`, custom keys). A PAT profile loses
  `token`, and a service-principal profile loses `client_id` and `client_secret`; both gain `auth_type`. So any
  other profile (also a same-name section in another case, or plaintext token storage) gets `profile_invalid`, the
  CLI does not run, and the hub says Bottle cannot sign the workspace in again. The CLI also adds
  `[__settings__] auth_storage = secure` and realigns the file, as for any login.
- **D5 to D7 of the director's plan** were not part of the S1-A brief. This ADR records the S1 decisions above.
  The owner or director must add any other D5 to D7 wording.

## Consequences

- One lamp. The old MOGs are workspaces with their name, colour, history and memory.
- The agent loop (slices 2 to 4) can rely on the allow-list in R8 and on the agent settings in the same file.
- No MOG window opens at launch. The brain opens the lamp (`lampMogId`) only on the talk key or a hub action,
  because one window is the cap.
