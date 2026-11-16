# Gate evidence template

AT-22 needs evidence for each release gate, and no unrun spike may be reported as a pass. Each
gate result is one file in `docs/evidence/`. Copy the block under "File format", then add the
section for the gate type. `scripts/check_gate_evidence.py` checks the files (#229).

## Rules

- One file per gate run, named after the gate, for example `docs/evidence/AT-07.md`,
  `docs/evidence/SP-03.md` or `docs/evidence/AT-22-signing-review.md`.
- **Gate** is an AT or SP ID of `research/final-requirements.md`. Review gates use `AT-22`
  (or `AT-37` for licences and supply chain).
- **Gate type** is one of: `spike`, `device`, `live`, `release`, `MVP exit`, `signing review`,
  `admin preflight`, `MDM keys`, `legal review`.
- **Result** is one of: `pass`, `fail`, `partial`, `not run`. If you did not run it, write `not run`.
  Never write `pass` for a gate that did not run.
- A `pass` needs a real **Date run** (YYYY-MM-DD), **Run by**, and a filled **Raw evidence** section.
- Never paste a token, a `.databrickscfg` file, a PAT, a client secret or customer data. Write the
  CLI profile name only. Redact host names if the workspace is private.
- **Scope** is optional. Write `part: <IDs> only` when the file covers only some rows of the gate
  (for example the latency file of #171). A ticked row needs one or more files without Scope `part`.
- To tick a row in `docs/release/RELEASE-CHECKLIST.md`, put the evidence path in its Evidence
  cell, for example `[log](docs/evidence/AT-07.md)`. The file must say Result `pass` for that AT.

## File format

```markdown
# Evidence: <gate ID> <short title>

| Field | Value |
|---|---|
| Gate | <AT-nn or SP-nn> |
| Gate type | <one gate type from the list above> |
| Result | <pass, fail, partial or not run> |
| Date run | <YYYY-MM-DD> |
| Run by | <person or agent role> |
| Build | <commit SHA, or installer file name and SHA-256> |
| Hardware | <reference device, for example Apple M1 8 GB, macOS 14.4> |
| Profile | <CLI profile name only, or "none"> |
| Requirements | <IDs that this run verifies, for example NFR-15a, NFR-15b> |
| Scope | <optional: "part: <IDs> only" when the file covers only some rows of the gate> |

## Method

<The procedure you ran, the commands and the settings.>

## Raw evidence

<Raw output, measurements, log extracts, or links to recordings and screenshots.>

## Result against the criterion

<The pass criterion from section 15 or 16, and the measured value for each part of it.>

## Follow-ups

<Issue numbers for failures, fallbacks and gaps.>
```

## Gate type: spike

For SP-01..SP-21 (section 16). Also keep the full report in `docs/spikes/SP-nn.md` and link it.

- Method: the spike method of section 16, the versions and the hardware.
- Raw evidence: every measurement (counts, p50 and p95, sizes), not only the summary.
- Result: `pass` only if every part of the pass criterion holds. On `fail` or `partial`, name the
  fallback that you chose and its issue. A spike that did not run stays `not run`.

## Gate type: device

For ATs with the gate `device` on the reference hardware: Apple M1 8 GB on macOS 14.4, Apple M1
on the current macOS, and a Windows 11 i5/16 GB laptop (section 15). One file per AT; list each
device with its own result.

- Hardware: the output of `sw_vers` and `sysctl -n machdep.cpu.brand_string hw.memsize`, or
  `Get-ComputerInfo -Property OsName,OsVersion,CsProcessors,CsTotalPhysicalMemory`.
- Raw evidence: screen recordings, logs and measurements. For talk-key tests, the HID rig log.

## Gate type: live

For ATs with the gate `live`, run against a live workspace with the `BOTTLE_TEST_PROFILE` fixture
(U2M CLI profile, no Lakebase; see `docs/tests/README.md`).

- Profile: the profile name only. Host: the workspace host, or "redacted".
- Raw evidence: the test output (`pnpm test`, `cargo test`), request IDs and timings. No tokens.

## Gate type: release

For ATs with the gate `release` (AT-22, AT-37). The AT-22 file links every other evidence file.

## Gate type: MVP exit

For AT-31 and AT-32. Link the device and live evidence of each step.

## Gate type: signing review

For NFR-15a..d: the macOS DMG is signed and notarized, the Windows installer is signed, and the
download page needs no sign-in. Gate `AT-22`.

- Raw evidence: `codesign --verify --deep --strict --verbose=2 <app>`, `spctl -a -vv <app>`,
  `xcrun stapler validate <dmg>`, `Get-AuthenticodeSignature <installer>`, the SHA-256 of each
  file and the download page URL.

## Gate type: admin preflight

For ADM-01a..ADM-15: Genie One on, the grants, network items and the CLI version. Gate `AT-22`.

- Raw evidence: `databricks grants get-effective ... --profile <your-profile>` output, the wizard
  probe results and the CLI version. A workspace admin runs this; never the agent.

## Gate type: MDM keys

For ADM-06 and the keys of Appendix C (for example `AllowCliInstall`, `CliPath`). Gate `AT-22`.

- Raw evidence: the profile or policy that sets each key, and for each key the app behaviour
  (for example "Set by your admin" with the lock icon, MB-15).

## Gate type: legal review

For ADM-18 and Table ADM-LEGAL (the customer legal pack before meeting sessions). Gate `AT-22`.

- Raw evidence: the reviewer, the date, the document versions and the decision. Do not paste the
  documents if they are confidential; link them.
