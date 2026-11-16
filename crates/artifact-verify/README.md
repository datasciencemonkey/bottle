# artifact-verify

Rust crate for Bottle (issue #160), a member of the root Cargo workspace. It verifies
downloaded artefacts against pinned SHA-256 digests.

## Requirements covered

| ID | What this crate does | Tests |
|---|---|---|
| SEC-20b | Streams each downloaded artefact (model file, Databricks CLI archive) through SHA-256 and compares it in constant time with the pinned digest. A mismatch deletes the file and returns a typed error, so the install stops. | `tests/verify.rs`, `tests/sums.rs` |
| W-17d | Parses the CLI release checksum file `databricks_cli_<ver>_SHA256SUMS` and checks the archive against it. A changed or unlisted archive stops the install and deletes the download. (Download, unpack and consent are CliRunner work.) | `tests/verify.rs`, `tests/sums.rs` |
| AT-37 | The "hashes are pinned" part of this acceptance test. | all |

The licence notices (SEC-21a, SEC-21b: `THIRD_PARTY_NOTICES.md` and the antigravity MIT header in ported files) are checked by `tests/third-party-notices.test.ts` (#228), so this crate does not check them again.

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

## How to test

```sh
cargo fmt --all --check
cargo clippy -p artifact-verify --all-targets --locked -- -D warnings
cargo nextest run -p artifact-verify --locked
```

App CI runs it on macOS and Windows with the rest of the workspace. Tests run on Linux, macOS and Windows; the "delete failed" branch uses a
read-only directory on Unix and a read-only file on Windows.

## Out of scope

* The notices file, the About screen and their checks (SEC-21a, SEC-21b): #228 (merged).
* Downloading, unpacking and consent for the Databricks CLI (W-17d): #161 (guided CLI
  install) on top of CliRunner (#16). They call `verify_against_sums` here.
* The model downloader and the real pinned digests of the model files: #45 (local
  Parakeet STT and consented model download). This crate gives the `PinnedArtifact`
  type and the check.
* SEC-20a (Rive with bundled assets) and the other AT-37 checks (brand, lockfile, offline build).
