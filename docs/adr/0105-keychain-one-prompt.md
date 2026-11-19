# ADR 0105: Bottle reads its keychain key once per launch, after the hub says why

- Status: accepted
- Issue: user request 2026-11-19 (MEM-08b, MB-08a)
- Date: 2026-11-19
- Owner: impl-keychain

## Context

At launch macOS asked for the login password: "bottle wants to use your confidential information
stored in … in your keychain". The user did not know why, and saw the prompt twice.

The keychain has one Bottle item: the local store key `com.databricks.bottle.local-store`
(account `memory.db`, login keychain for builds with no keychain entitlement). Its access list
names each Bottle build. An unsigned or ad-hoc build has a new signature after each rebuild, so
macOS asks again. The access list shows two entries for each build path, so each build was
allowed twice. In the code, `consent_host::install` opened the store at setup, before any window
showed. That is one read (`SecItemCopyMatching`). There is no second Bottle item. The 11
`databricks-cli` items belong to the Databricks CLI, and their access list trusts
`/usr/bin/security`, so a CLI read does not show this prompt.

The code had no guard against more reads in the same launch. After a Deny, each brain store call
(the consent check before each capture, the history reads) opened the store again, and each open
read the key again. That is a prompt for each call.

## Decisions

1. **One read per launch.** `os_key_vault()` wraps the OS vault in `CachedKeyVault`. The first key
   that loads, or that the vault stores, stays in process memory. Every open uses it. A failed
   read is not kept, so Try again reads again.
2. **No read at setup.** `consent_host::install` prepares the store, the records and R14, but
   does not open the store. `SqlCipherLocalStore` opens on its first use. The R14 content-free
   expiries run once, right after the first open (`on_first_open`).
3. **The reason first.** On macOS (a Mac user agent inside Tauri), the brain store gate holds
   each store call until the hub shows this line: "Bottle keeps your history encrypted on this
   Mac. macOS will ask for your password once so Bottle can unlock its key. Choose Always Allow."
   The gate also opens on a talk-key press, for a run with no hub.
4. **One probe.** The first store call runs alone. The others wait for its result. After a
   refusal (`consent_storage_failed`, `host_failed`), every store call fails at once with no
   prompt. The hub shows "History is off: Bottle could not unlock its key." with Try again.
5. **One item.** Bottle has one keychain item, so no migration is needed.

## Consequences

- The consent records are in the same store (ADR 0071). So after a Deny, the notice gate stays
  closed: there is no capture before a stored acceptance (MB-08a). "Bottle works without local
  history" would need a session-only acceptance. That is a requirements decision; this ADR does
  not make it.
- A signed build where the user chose Always Allow does not ask again. AGENTS.md section 4 says
  so.
