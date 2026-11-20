# ADR 0107: link_open: the one ARC-6 exception

- Status: accepted
- Issue: refs #523 (PR #607)
- Date: 2026-11-19
- Owner: impl-genie-links

## Context

ARC-6 says: "Commands take profile names, route IDs and statement IDs, not URLs or headers."
A Genie answer holds links, and Genie One returns a `deep_link` to the conversation in the
workspace. A user must be able to open them in the browser. A view cannot open a URL (ARC-4: it
only listens and calls `view_emit`), and the brain has no browser capability. So Rust must open the
link, and that needs a URL argument on one command.

## Decision

Add the command `link_open {profile, url}`. It is the one command that takes a URL.

- **Approval.** The project owner approved this exception on 2026-11-19, in a comment on PR #607
  ("CONTRACT AUTHORIZATION (ARC-6 exception)"). It covers the contract and test edits for this
  command only.
- **Scope: brain only.** The command row lists only the Brain window class. No view capability has
  `allow-link-open`. A view sends a `view_emit` click `{target: 'link_open', url}`. The brain
  finds the workspace from its own data (the MOG's home profile for a card, the exchange it
  listed for the hub), checks the URL, and calls `link_open`. `ipc_capabilities.rs` checks that
  views cannot open URLs, and that the opener and shell plugins are not dependencies. The opener is
  the existing `SystemOpener`: one argument, no shell.
- **Three checks, one rule.** The view decides what looks clickable. The brain checks the click
  again. Rust checks last and is the strict one: it trusts no caller. All three use the same rule.
  `src/shared/link-cases.json` holds allow and refuse cases. A TS test and a Rust test run it, so a
  link that looks clickable is never one that Rust refuses.
- **Rust validation (`mog_core::link_open::validate_link`).**
  - The URL is ASCII, at most 2048 bytes, and starts with `https://` (any case).
  - It has no control character, space, `\`, `<`, `>` or `"`.
  - The authority (up to the first `/`, `?` or `#`) is a bare host of `[A-Za-z0-9.-]`. So there is
    no userinfo, no port (also not `:443`), no `%` escape, no IPv6 and no leading, trailing or
    doubled dot.
  - The host equals the workspace host that T7 gives for `profile` (never a value from the
    caller), or the host of an allowed app entry. An allowed entry must be `https://host[/path]`
    and trusts that host. Nothing supplies entries today.
  - Rust rebuilds `https://<lower-case host><rest>` and passes it to the opener.
- **`deep_link` trust.** "Open in Databricks" uses the `deep_link` that `genie_poll_response`
  returned, stored in `genie_exchange.deep_link`. It is shown only if it passes the same rule for
  the workspace host. Bottle never builds a URL from conversation or response ids: Databricks does
  not document the `/one/chat/threads/...` route as a stable contract. A null `deep_link` shows no
  button.
- **Links from an answer.** An answer is untrusted text. A link in it is clickable only if it
  passes the rule. A link to any other host is plain text with its host shown.

## Accepted risk: open redirect on workspace paths

Any path on the workspace host is trusted. If a workspace endpoint redirected to a URL given in a
query parameter, an answer that an attacker influences (Genie output can repeat table data) could
send the user to another site after the click. We accept this for now. The click opens the user's
own workspace host in their own browser, so the browser address bar shows that host first. A path
allow-list (`/one/`, `/sql/`, `/explore/`, `/editor/`, `/jobs/`) would break valid links that
Genie or users add, and Databricks publishes no such list. We have no report of such a redirect.
Revisit this if one is found.

## What needs a new ADR

- Any other command that takes a URL, or `link_open` in a window other than the brain.
- A view that opens a URL itself, or the opener, shell or http plugin as a dependency.
- Trust for a host other than the profile's workspace host, such as a real `AllowedAppUrls`
  source, a port, an IDN host or a path allow-list.
- Building a Databricks URL from ids.

## Consequences

- ARC-6 (`research/final-requirements.md`) names this ADR as its one exception. The two ARC-6
  tests (`ipc_contract.rs`, `token_isolation.rs`) name it too.
- One more brain-only command and capability. No migration.
