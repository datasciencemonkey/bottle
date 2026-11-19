# ADR 0099: The guided CLI install uses a pinned release and its pinned checksum file

- Status: accepted
- Issue: #161 (W-17a, W-17b, W-17c, W-17d, W-17e, W-17f, SEC-20b, U16)
- Date: 2026-11-18
- Owner: impl-161 (agent-lead-5)

## Context

U16 says that a user with no Databricks CLI, or a CLI older than 1.17, must never need a
terminal. W-17d asks for a check against the release SHA-256 checksum file. SEC-20b asks for a
check against a pinned SHA-256. The issue left these points open:

1. Which release Bottle installs: the latest release or a fixed release.
2. How one pin covers every OS and CPU.
3. Where the install and its errors cross the IPC, because no new IPC row is allowed.

## Decisions

1. Bottle installs one fixed release, `PINNED_CLI_VERSION` 1.26.0, in `cli_runner::install`.
   A new release needs a code change and a review. Bottle never asks GitHub for the latest
   release.
2. Bottle pins the SHA-256 of the release checksum file `databricks_cli_1.26.0_SHA256SUMS`
   (`PINNED_SUMS_SHA256`). The install checks the checksum file against the pin first. Then it
   checks the archive against its line in the checksum file. So one pin covers every archive.
3. The archive is the `.zip` of the OS and CPU (`darwin`, `windows`, `linux`; `amd64`,
   `arm64`). The install unpacks only the top-level `databricks` (`databricks.exe`) entry.
4. Each install uses a new, empty folder `Bottle/downloads/<unique>`. Each failure deletes this
   folder. The install moves the CLI into `Bottle/bin` only after both checks pass.
5. The real download is HTTPS only, with no token and no cookie. It follows a redirect only to
   `github.com`, `objects.githubusercontent.com` or `release-assets.githubusercontent.com`.
6. R5 looks in the Bottle bin folder first, after the MDM `CliPath`. So R5 uses the Bottle copy,
   and an older CLI on PATH stays unchanged (W-17f).
7. The existing `cli_install {consent:true}` row runs the install. The host routes `cli_status`
   and `cli_install` to the R5 status component. An install failure crosses as `host_failed`,
   because `ERROR_CODES` has no install codes. The hub shows one general text.
8. The MDM key `AllowCliInstall` (W-17g) and the admin install text are out of scope (user
   decision). A follow-up issue adds them.

## Consequences

- An update of the pinned release needs two new constants and a review.
- The hub cannot tell a checksum failure from a network failure. A contract change can add
  install codes later.
