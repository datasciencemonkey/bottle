# artifact-verify

Standalone Rust crate for Bottle (issue #160). It verifies downloaded artefacts
against pinned SHA-256 digests and checks the third-party licence notices.

## Requirements covered

| ID | What this crate does | Tests |
|---|---|---|
| SEC-20b | Streams each downloaded artefact (model file, Databricks CLI archive) through SHA-256 and compares it in constant time with the pinned digest. A mismatch deletes the file and returns a typed error, so the install stops. | `tests/verify.rs`, `tests/sums.rs` |
| W-17d | Parses the CLI release checksum file `databricks_cli_<ver>_SHA256SUMS` and checks the archive against it. A changed or unlisted archive stops the install and deletes the download. (Download, unpack and consent are CliRunner work.) | `tests/verify.rs`, `tests/sums.rs` |
| SEC-21a | Finds files marked as ported from antigravity-bricks-plugin and checks that each keeps the MIT header of commit a31fd2c. A test scans the whole repository. | `tests/notices.rs` |
| SEC-21b | `THIRD_PARTY_NOTICES.md` at the repo root holds every notice of requirements section 20.1; a test reads it and checks each one. | `tests/notices.rs` |
| AT-37 | The "notices exist" and "hashes are pinned" parts of this acceptance test. | all |

## API

```rust
use artifact_verify::{verify_file, verify_pinned, verify_against_sums,
    PinnedArtifact, Sha256Digest, Sha256Sums, VerifyError};

// Pinned model file.
const VAD: PinnedArtifact = PinnedArtifact { file_name: "silero_vad.onnx", sha256_hex: "<64 hex>" };
verify_pinned(path, &VAD)?;                       // Err(Mismatch { deleted: true, .. }) on a change

// Databricks CLI archive.
let sums = Sha256Sums::parse(&sums_text)?;        // text of databricks_cli_<ver>_SHA256SUMS
verify_against_sums(path, &sums, "databricks_cli_<ver>_<os>_<arch>.zip")?;
```

* `Sha256Digest`: `from_hex` (64 hex chars, any case), `to_hex`, `ct_eq` (constant time;
  `==` uses it too).
* `sha256_reader` / `sha256_file`: streaming hash with a 64 KiB buffer.
* `VerifyError`: `Io`, `BadPin`, `Mismatch { expected, actual, deleted }`,
  `NotListed { deleted }`. Every variant except `Io` deletes the download; `deleted`
  is false only when the delete failed (the install must still stop).
* `sums::sums_file_name(version)` gives `databricks_cli_<ver>_SHA256SUMS`.
* `notices::missing_notices(text)`: components of section 20.1 absent from a notices text.
* `notices::check_ported_text` / `check_ported_file` / `scan_ported_files(root)`:
  SEC-21a header check. A file is "marked as ported" when one of its lines, after a
  comment marker (`//`, `#`, `/* */`, `<!-- -->`, ...), is exactly the
  `notices::PORTED_MARKER` text. The marked file must then contain the full
  `notices::ANTIGRAVITY_MIT_HEADER` (comment markers and white space are ignored).

## How to test

```sh
cd crates/artifact-verify
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

The crate has its own empty `[workspace]` table and its own `Cargo.lock`, so it builds
alone. Tests run on Linux, macOS and Windows; the "delete failed" branch uses a
read-only directory on Unix and a read-only file on Windows.

## Out of scope

* About screen display of the notices (SEC-21b UI part): UI work, owned by #228
  ("Add THIRD_PARTY_NOTICES and the About licence screen"); it must render
  `THIRD_PARTY_NOTICES.md` or the same texts.
* Downloading, unpacking and consent for the Databricks CLI (W-17d): #161 (guided CLI
  install) on top of CliRunner (#16). They call `verify_against_sums` here.
* The model downloader and the real pinned digests of the model files: #45 (local
  Parakeet STT and consented model download). This crate gives the `PinnedArtifact`
  type and the check.
* SEC-20a (Rive with bundled assets) and the other AT-37 checks (brand, lockfile, offline build).
