# ADR 0049: Local Parakeet STT packaging, the consent gate and the model download

- Status: Accepted
- Date: 2026-11-16
- Issue: #45 (depends on #43 wizard, #44 SP-04; next: #46 SttEngine)
- Owner: mog-b2

## Context

CAP-48 to CAP-50b and D78 set local STT: the official `sherpa-onnx` 2.0.x crate, the CPU EP, one
thread for each P-core, vendored hash-pinned native archives, no build-time download, and a
487 MB model that downloads only after consent, only from the sherpa-onnx release or the HF
mirror, with resume and a pinned SHA-256. The issue left these points open: how CI builds without
the 120 MB Windows archive, which HF mirror, how the wizard (#43, still open) gives consent, and
what the build does when the archive is missing.

## Decisions

1. **Cargo feature `local-stt` on `mog-core`, off by default.** It turns on `sherpa-onnx =
   "=2.0.1"` (static), `ureq`, `bzip2` and `tar` (the same crates that the `sherpa-onnx-sys` build
   script already uses) and the module `stt_local` (recognizer, HTTPS fetcher, archive extract).
   The default build, and so App CI today, links no native STT library and fetches nothing. The
   release build and the app turn the feature on. An offline CI job for `local-stt` is a CI change
   (follow-up issue; this PR does not edit workflows).
2. **The build links only the vendored archive (CAP-50a, CAP-50b).** Three parts:
   - `.cargo/config.toml` sets `SHERPA_ONNX_ARCHIVE_DIR` to `vendor/sherpa-onnx` (gitignored).
     So `sherpa-onnx-sys` never takes its download branch: with no archive there, it stops with
     "does not contain expected archive". A value in the environment wins over this one.
   - `sherpa-onnx-sys` is a direct optional dependency of `mog-core`. It has `links`, so Cargo
     runs its build script before ours.
   - `crates/mog-core/build.rs` (logic in `build_support/sherpa_vendor.rs`, tested in
     `tests/stt_build_check.rs`) refuses `SHERPA_ONNX_LIB_DIR`. It checks the archive against
     `crates/mog-core/sherpa-onnx-archives.sha256`. Then it checks that the folder that
     `sherpa-onnx-sys` links (`<target>/sherpa-onnx-prebuilt/<stem>/lib`) is byte-equal to the
     archive's `lib/` files, so an old unchecked download in the cache stops the build. The build
     log line is `CAP-50a vendored sherpa-onnx archive <name> sha256 <hash> OK; linked lib dir
     <dir> matches it (<n> files)`.
   - Pinned targets: macOS arm64 and x64, Windows x64 (`static-MT-Release`), Linux x64.
   - `CARGO_NET_OFFLINE` does not stop a build script from opening a socket. The CI proof (#414)
     must use a clean target folder with the network blocked at the OS level.
3. **`scripts/vendor_sherpa_onnx.py` fetches the archives once** into `vendor/sherpa-onnx` from
   the v2.0.1 GitHub release, with resume, and checks the same pin file. After that, `cargo build --offline` works with no
   network (verified on macOS with a dead proxy and `CARGO_NET_OFFLINE=true`).
4. **Thread count (CAP-48).** `stt::perf_core_count()`: macOS `hw.perflevel0.physicalcpu`;
   Windows the physical cores of the highest `EfficiencyClass`
   (`GetLogicalProcessorInformationEx`); other systems the available parallelism.
   `LocalSttConfig::for_this_device()` fixes the provider to `cpu`.
5. **Model sources (CAP-49b).** The GitHub release archive
   `asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2` is the default. The HF mirror is
   `csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` at the fixed commit `ed5d745`, file by
   file (it has no archive). `is_allowed_url` accepts only plain file names under these two
   prefixes. Handy downloads Parakeet from its own host and HF in GGUF form; Bottle does not use
   those (the requirements name sherpa-onnx). CAP-49b allowlists the URL that Bottle requests.
   The HTTPS client follows the hosts' redirects to their CDNs (GitHub release assets, HF LFS).
   The integrity of the bytes comes from the pins (CAP-49c), not from the redirect host.
6. **Pins (CAP-49c).** The archive size and SHA-256 (the GitHub asset digest), and the size and
   SHA-256 of the 4 model files (equal to the HF LFS digests). Each download and each extracted
   file is checked. A mismatch deletes the file and stops the install. The extract writes only the
   4 pinned names under the archive's top folder.
7. **Resume.** A download writes `<root>/<file>.partial`. The next install sends
   `Range: bytes=<n>-`. A `206` with a matching `Content-Range` appends; a `200` restarts the file.
8. **The consent gate is an interface (CAP-49a).** `stt_model::ConsentGate` returns
   `NotAsked`, `Declined` or `Granted`. `ModelInstaller::install` checks it before any request
   or file write. The wizard STT step (MB-07, #43) implements the gate, shows `status()` and the
   `Progress` callback, and can cancel. Wiring the wizard is a follow-up issue. Errors are
   content-free codes (`stt_consent_required`, `stt_hash_mismatch`, ...).

## Consequences

- From Handy (MIT): the `.partial` resume file, the `Content-Range` parser and the
  delete-on-mismatch hash check, in `stt_model.rs` (attribution header and THIRD_PARTY_NOTICES).
- Not yet done: the wizard wiring (#43 follow-up), the offline `local-stt` CI job (CI owner), the
  Windows link check of the MT archive with the default Rust CRT (`needs:windows`), and the
  engine lifetime and VAD decode (#46).
- NFR-05 (RSS 1.0 GB) fails on SP-04 (#282). This ADR does not change it.
- Dev tool: `cargo run -p mog-core --features local-stt --release --example stt_install --
  --models <dir> --source github --i-consent --wav <16 kHz mono WAV>`.
