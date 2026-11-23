"""Tests for the app CI workflow and the pinned lockfiles (NFR-13a, NFR-15a/b, SEC-26, AT-37, #227).

App CI runs on Windows and Linux on every event, except that the Ubuntu leg of the checks job skips pull
requests and a docs-only pull request skips App CI. macOS is not in CI: GitHub's hosted macOS queue
held runs for hours, so the developer runs scripts/pre-push.sh on a Mac before each push, and
scripts/package-macos.sh builds, signs and notarizes the DMG (ADR 0045).

Run:  uv run --no-project --with pytest --with pyyaml pytest scripts/tests -q
"""
import json
import os
import re
import shutil
import subprocess

import pytest
import yaml
from conftest import ROOT

WORKFLOWS = ROOT / ".github" / "workflows"
APP_CI = WORKFLOWS / "app-ci.yml"
PRE_PUSH = ROOT / "scripts" / "pre-push.sh"
PACKAGE_MACOS = ROOT / "scripts" / "package-macos.sh"
CI_OSES = {"ubuntu-24.04", "windows-2025"}
SHA_PIN = re.compile(r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")


@pytest.fixture(scope="module")
def app_ci():
    assert APP_CI.exists(), f"{APP_CI.relative_to(ROOT)} does not exist"
    return yaml.safe_load(APP_CI.read_text(encoding="utf-8"))


def steps(job):
    return job.get("steps", [])


def runs(job):
    return "\n".join(s.get("run", "") for s in steps(job))


def all_runs(wf):
    return "\n".join(runs(j) for j in wf["jobs"].values())


# The matrix is chosen per event:
# `github.event_name == 'pull_request' && fromJSON('<PR JSON>') || fromJSON('<other JSON>')`.
EVENT_MATRIX = re.compile(
    r"github\.event_name == 'pull_request' && fromJSON\('(\[.*?\])'\) \|\| fromJSON\('(\[.*?\])'\)")
EVENTS = ("pull_request", "push", "schedule", "workflow_dispatch")
MATRIX_JOBS = ("checks", "offline-build", "golden-set", "package")


def matrix_entries(job, event):
    """The matrix entries (dicts with at least `os`) that the job runs for one event."""
    matrix = job.get("strategy", {}).get("matrix", {})
    for key in ("os", "include"):
        value = matrix.get(key)
        if isinstance(value, str):
            m = EVENT_MATRIX.search(value)
            assert m, f"matrix.{key} is not an event-chosen fromJSON expression: {value}"
            value = json.loads(m.group(1) if event == "pull_request" else m.group(2))
        if value:
            return [{"os": i} if key == "os" else i for i in value]
    return []


def matrix_oses(job, event=None):
    """The runner OSes of the job for one event, or for all events when event is None."""
    return {e.get("os") for ev in ([event] if event else EVENTS) for e in matrix_entries(job, ev)}


def test_nfr_13a_app_ci_runs_on_windows_and_linux_except_ubuntu_checks_on_pull_requests(app_ci):
    """[NFR-13a] Offline build and golden set run on Windows and Linux on every event. Checks does too,
    except that a pull request runs it on Windows only (ADR 0045)."""
    for event in EVENTS:
        for name in ("offline-build", "golden-set"):
            oses = matrix_oses(app_ci["jobs"][name], event)
            assert oses == CI_OSES, f"{name} on {event} runs on {oses}, not {CI_OSES}"
        oses = matrix_oses(app_ci["jobs"]["checks"], event)
        expected = {"windows-2025"} if event == "pull_request" else CI_OSES
        assert oses == expected, f"checks on {event} runs on {oses}, not {expected}"


def test_nfr_13a_checks_keeps_its_job_name_pattern(app_ci):
    """[NFR-13a] The checks job name stays `Checks (<os>)`."""
    assert app_ci["jobs"]["checks"]["name"] == "Checks (${{ matrix.os }})"


def pr_paths(app_ci):
    on = app_ci.get("on", app_ci.get(True))
    return on["pull_request"]["paths"]


def pr_runs_for(app_ci, changed):
    """True when a pull request that changes only `changed` starts App CI (GitHub `paths` rules)."""
    def match(pattern, path):
        regex = re.escape(pattern).replace(r"\*\*/", "(?:.*/)?").replace(r"\*\*", ".*").replace(r"\*", "[^/]*")
        return re.fullmatch(regex, path) is not None

    def included(path):
        result = False
        for pattern in pr_paths(app_ci):
            if match(pattern.lstrip("!"), path):
                result = not pattern.startswith("!")
        return result

    return any(included(p) for p in changed)


def test_nfr_13a_docs_only_pull_requests_skip_app_ci(app_ci):
    """[NFR-13a] A pull request that changes only docs, research, markdown or requirements.html skips App CI."""
    on = app_ci.get("on", app_ci.get(True))
    assert "paths-ignore" not in on["pull_request"], "paths and paths-ignore cannot be mixed"
    for changed in (["docs/adr/0045-ci-macos-local.md"], ["docs/VISION.md"], ["research/message-board.md"],
                    ["README.md", "AGENTS.md"], ["requirements.html"]):
        assert not pr_runs_for(app_ci, changed), f"a PR that changes only {changed} must skip App CI"


def test_nfr_13a_pull_requests_run_app_ci_for_code_and_files_that_tests_read(app_ci):
    """[NFR-13a] Code, workflows and the docs files that tests read still start App CI on a pull request."""
    for changed in (["src/main.ts"], ["crates/mog-core/src/lib.rs"], [".github/workflows/app-ci.yml"],
                    ["docs/user/where-your-data-goes.md"], ["research/final-requirements.md"],
                    ["THIRD_PARTY_NOTICES.md"], ["docs/tests/TEST-LEDGER.md"],
                    ["docs/release/RELEASE-CHECKLIST.md"], ["docs/spikes/SP-01.md"],
                    ["docs/VISION.md", "src/main.ts"]):
        assert pr_runs_for(app_ci, changed), f"a PR that changes {changed} must run App CI"


def test_nfr_13a_every_markdown_file_in_a_code_folder_triggers_app_ci(app_ci):
    """[NFR-13a] No tracked .md file under scripts, src, tests, crates or src-tauri skips App CI."""
    out = subprocess.run(["git", "ls-files", "scripts", "src", "tests", "crates", "src-tauri"], cwd=ROOT,
                         capture_output=True, text=True, check=True).stdout.splitlines()
    md = [f for f in out if f.endswith(".md")]
    assert md, "no markdown fixture found; the test checks nothing"
    skipped = [f for f in md if not pr_runs_for(app_ci, [f])]
    assert not skipped, f"these files would skip App CI: {skipped}"


def test_nfr_13a_paths_filter_applies_to_pull_requests_only(app_ci):
    """[NFR-13a] Push to main, the schedule and dispatch have no path filter, so they run everything."""
    on = app_ci.get("on", app_ci.get(True))
    assert on["push"] == {"branches": ["main"]}
    assert on["schedule"] and on["workflow_dispatch"] is None
    for trigger in ("push", "schedule", "workflow_dispatch"):
        value = on[trigger]
        assert not isinstance(value, dict) or not {"paths", "paths-ignore"} & set(value)
    repo = yaml.safe_load((WORKFLOWS / "repo-checks.yml").read_text(encoding="utf-8"))
    assert "pull_request" in repo.get("on", repo.get(True)), "repo-checks.yml must run on every pull request"
    assert repo.get("on", repo.get(True))["pull_request"] is None, "repo-checks.yml must have no path filter"


def test_nfr_13a_app_ci_has_no_macos_runner(app_ci):
    """[NFR-13a] No App CI job runs on a hosted macOS runner; the macOS gate is scripts/pre-push.sh."""
    for name, job in app_ci["jobs"].items():
        oses = matrix_oses(job) | {str(job.get("runs-on", ""))}
        assert not any("macos" in o.lower() for o in oses if o), f"{name} runs on macOS: {oses}"
    assert "runner.os == 'macOS'" not in APP_CI.read_text(encoding="utf-8"), "a macOS-only step is left in CI"
    assert PRE_PUSH.exists(), "scripts/pre-push.sh does not exist"


def test_nfr_13a_linux_jobs_install_the_tauri_system_libraries_first(app_ci):
    """[NFR-13a] Linux jobs that compile Rust install WebKitGTK and the other Tauri libraries first."""
    for name, job in app_ci["jobs"].items():
        if not any(o and o.startswith("ubuntu") for o in matrix_oses(job)):
            continue
        body = steps(job)
        builds = [i for i, s in enumerate(body) if re.search(r"\bcargo\b|tauri build", s.get("run", ""))]
        if not builds:
            continue
        apt = [i for i, s in enumerate(body)
               if "apt-get install" in s.get("run", "") and "libwebkit2gtk-4.1-dev" in s.get("run", "")
               and "runner.os == 'Linux'" in str(s.get("if", ""))]
        assert apt, f"{name}: no Linux-only step installs libwebkit2gtk-4.1-dev"
        assert apt[0] < builds[0], f"{name}: the Tauri libraries are installed after the first cargo step"


@pytest.fixture(scope="module")
def pre_push():
    assert PRE_PUSH.exists(), "scripts/pre-push.sh does not exist"
    return PRE_PUSH.read_text(encoding="utf-8")


def test_nfr_13a_pre_push_runs_the_ci_checks_on_macos(app_ci, pre_push):
    """[NFR-13a, NFR-23a] pre-push.sh runs the commands of the CI checks job, with requirement coverage."""
    for cmd in ("pnpm install --frozen-lockfile", "pnpm lint", "pnpm format:check", "pnpm typecheck",
                "pnpm test:ci", "cargo fmt --all --check",
                "cargo clippy --workspace --all-targets --locked -- -D warnings",
                "cargo nextest run --workspace --locked --profile ci", "reports/junit-rust.xml",
                "cargo test --workspace --locked --doc", "tools/ci_checks.py", "pytest scripts/tests",
                "reports/junit-py.xml", "scripts/req_coverage.py", "reports/requirements-coverage.md"):
        assert cmd in pre_push, f"pre-push.sh does not run {cmd!r}"


def test_nfr_13a_pre_push_checks_the_pinned_nextest_version(app_ci, pre_push):
    """[NFR-13a] pre-push.sh fails when cargo-nextest is not the version that App CI pins (0.9.152)."""
    assert app_ci["env"]["NEXTEST_VERSION"] == "0.9.152"
    assert "NEXTEST_VERSION" in pre_push and "app-ci.yml" in pre_push, "pre-push.sh does not read the CI pin"
    assert "cargo nextest --version" in pre_push
    assert "cargo test --workspace --locked\n" not in pre_push, "pre-push.sh must not fall back to cargo test"


def test_at_37_pre_push_builds_offline_from_the_lockfiles(pre_push):
    """[AT-37] pre-push.sh fetches with the locked lockfiles, then builds with the network off."""
    for cmd in ("cargo fetch --locked", "pnpm install --offline --frozen-lockfile", "pnpm build",
                "CARGO_NET_OFFLINE=true", "cargo build --workspace --locked --offline"):
        assert cmd in pre_push, f"pre-push.sh does not run {cmd!r}"


def test_sec_26_pre_push_runs_the_adversarial_golden_set(pre_push):
    """[SEC-26] pre-push.sh runs the adversarial golden set on macOS (TS SEC-26 and Rust sec_26)."""
    assert 'vitest run -t "SEC-26" --passWithNoTests' in pre_push
    assert "cargo test --locked -p mog-core -p mog-core-ffi sec_26" in pre_push


def test_nfr_13a_pre_push_prints_a_summary_and_fails_nonzero(pre_push):
    """[NFR-13a] pre-push.sh prints PASS or FAIL for each step and exits non-zero when one fails."""
    assert "Result: PASS" in pre_push and "Result: FAIL" in pre_push
    assert re.search(r"Result: FAIL\"\s*\n\s*exit 1", pre_push)


def test_nfr_13a_app_ci_has_a_nightly_schedule_and_all_triggers(app_ci):
    """[NFR-13a] App CI runs on pull_request, push to main, a nightly schedule off the hour, and dispatch."""
    on = app_ci.get("on", app_ci.get(True))
    assert "pull_request" in on and "workflow_dispatch" in on
    assert on["push"]["branches"] == ["main"]
    crons = [c["cron"] for c in on.get("schedule") or []]
    assert crons, "no schedule trigger"
    for cron in crons:
        minute, hour, dom, month, dow = cron.split()
        assert minute.isdigit() and int(minute) != 0, f"cron {cron!r} runs on the hour"
        assert hour.isdigit() and (dom, month, dow) == ("*", "*", "*"), f"cron {cron!r} is not nightly"


def test_nfr_13a_app_ci_cancels_superseded_pr_runs(app_ci):
    """[NFR-13a] A new push to the same PR ref cancels the run in progress."""
    assert app_ci["concurrency"]["cancel-in-progress"] is True
    assert "github.ref" in app_ci["concurrency"]["group"]


def test_nfr_13a_app_ci_has_read_only_permissions_and_timeouts(app_ci):
    """[NFR-13a] The workflow reads contents only, and each job has a timeout."""
    assert app_ci["permissions"] == {"contents": "read"}
    for name, job in app_ci["jobs"].items():
        assert isinstance(job.get("timeout-minutes"), int), f"{name} has no timeout-minutes"


def test_nfr_23a_app_ci_runs_lint_tests_and_requirement_coverage(app_ci):
    """[NFR-23a] The checks job lints, type-checks and tests TS and Rust, writes JUnit, and runs coverage."""
    text = runs(app_ci["jobs"]["checks"])
    for cmd in ("pnpm install --frozen-lockfile", "pnpm lint", "pnpm format:check", "pnpm typecheck",
                "pnpm test:ci", "cargo fmt --all --check", "-- -D warnings", "cargo nextest run",
                "reports/junit-rust.xml", "scripts/req_coverage.py", "reports/junit-ts.xml",
                "reports/requirements-coverage.md", "reports/requirements-coverage.json"):
        assert cmd in text, f"checks job does not run {cmd!r}"


def test_at_37_every_action_is_pinned_to_a_commit_sha():
    """[AT-37] Every third-party action in every workflow is pinned to a 40-character commit SHA."""
    for path in sorted(WORKFLOWS.glob("*.yml")):
        wf = yaml.safe_load(path.read_text(encoding="utf-8"))
        for job in wf["jobs"].values():
            for step in steps(job):
                if "uses" in step:
                    assert SHA_PIN.match(step["uses"]), f"{path.name}: {step['uses']} is not pinned to a SHA"


def test_at_37_offline_build_uses_locked_lockfiles(app_ci):
    """[AT-37] The offline build fetches with the locked lockfiles, then builds with the network off."""
    text = runs(app_ci["jobs"]["offline-build"])
    assert "pnpm install --frozen-lockfile" in text
    assert "cargo fetch --locked" in text
    assert re.search(r"cargo build .*--locked.*--offline|cargo build .*--offline.*--locked", text)
    assert "pnpm install --offline --frozen-lockfile" in text
    for line in text.splitlines():
        if line.strip().startswith(("cargo build", "cargo test", "cargo clippy")):
            assert "--locked" in line, f"cargo without --locked: {line.strip()}"


def test_at_37_cargo_lock_pins_a_checksum_for_each_registry_crate():
    """[AT-37] Each crates.io package in Cargo.lock has a checksum."""
    blocks = (ROOT / "Cargo.lock").read_text(encoding="utf-8").split("[[package]]")
    registry = [b for b in blocks if 'source = "registry+' in b]
    assert registry, "Cargo.lock has no registry packages"
    missing = [b.split('"')[1] for b in registry if "checksum = " not in b]
    assert not missing, f"no checksum for: {missing}"


def test_at_37_pnpm_lock_pins_an_integrity_hash_for_each_package():
    """[AT-37] Each package resolution in pnpm-lock.yaml has an integrity hash."""
    lock = yaml.safe_load((ROOT / "pnpm-lock.yaml").read_text(encoding="utf-8"))
    packages = lock.get("packages") or {}
    assert packages, "pnpm-lock.yaml has no packages"
    missing = [name for name, meta in packages.items() if "integrity" not in (meta.get("resolution") or {})]
    assert not missing, f"no integrity hash for: {missing}"


def test_nfr_16b_pnpm_lock_has_no_rive_webgl():
    """[NFR-16b, AT-37] The lockfile has no `@rive-app/webgl` package."""
    lock = yaml.safe_load((ROOT / "pnpm-lock.yaml").read_text(encoding="utf-8"))
    names = {re.sub(r"(?<=.)@[^@]+$", "", name) for name in (lock.get("packages") or {})}
    assert "@rive-app/webgl" not in names


APPLE_SIGNING = {"APPLE_CERTIFICATE": "x", "APPLE_CERTIFICATE_PASSWORD": "x", "APPLE_SIGNING_IDENTITY": "x"}
APPLE_NOTARY = {"APPLE_ID": "x", "APPLE_PASSWORD": "x", "APPLE_TEAM_ID": "x"}


def package_macos_plan(extra):
    bash = shutil.which("bash")
    if bash is None:
        pytest.skip("bash is not on PATH")
    env = {k: v for k, v in os.environ.items() if not k.startswith("APPLE_")}
    env.update(extra)
    proc = subprocess.run([bash, str(PACKAGE_MACOS), "--plan"], cwd=ROOT, env=env,
                          capture_output=True, text=True)
    assert proc.returncode == 0, proc.stderr
    return dict(line.split(": ", 1) for line in proc.stdout.splitlines() if ": " in line)


@pytest.mark.parametrize("extra, sign, notarize", [
    ({}, "skip", "skip"),
    (APPLE_SIGNING, "yes", "skip"),
    (APPLE_NOTARY, "skip", "skip"),
    ({**APPLE_SIGNING, **APPLE_NOTARY}, "yes", "yes"),
])
def test_nfr_15a_macos_signing_and_notarization_skip_without_secrets(extra, sign, notarize):
    """[NFR-15a] package-macos.sh signs only with the APPLE_* signing secrets, and notarizes only when signed."""
    assert PACKAGE_MACOS.exists(), "scripts/package-macos.sh does not exist"
    plan = package_macos_plan(extra)
    assert plan.get("sign") == sign and plan.get("notarize") == notarize, plan


def test_nfr_15a_package_macos_builds_notarizes_and_hashes_the_dmg():
    """[NFR-15a, NFR-15d] package-macos.sh builds the DMG, notarizes and staples it, and writes a SHA-256 file."""
    text = PACKAGE_MACOS.read_text(encoding="utf-8")
    for cmd in ("pnpm tauri build --debug --bundles dmg --features local-stt -- --locked",
                "xcrun notarytool submit", "xcrun stapler staple", "shasum -a 256", ".sha256"):
        assert cmd in text, f"package-macos.sh does not run {cmd!r}"


def test_413_package_macos_builds_with_local_stt_and_vendors_it_first():
    """[#413, #45, ADR 0104] the packaged DMG gets local STT: vendor sherpa-onnx first (when it
    is missing), then build with --features local-stt, so a new user needs no developer command.
    """
    text = PACKAGE_MACOS.read_text(encoding="utf-8")
    assert "--features local-stt" in text
    assert "vendor_sherpa_onnx.py" in text
    assert "uv run --no-project python scripts/vendor_sherpa_onnx.py" in text
    # Vendoring is skipped when the archive is already there (CAP-50a is idempotent either way).
    vendor_line = next(line for line in text.splitlines() if "uv run --no-project" in line)
    guard = text[: text.index(vendor_line)]
    assert "tar.bz2" in guard, "package-macos.sh does not check for the archive before vendoring"


def test_nfr_15b_windows_signing_skips_without_secrets(app_ci):
    """[NFR-15b] The Windows sign step reads secrets and runs only when the secrets exist."""
    job = app_ci["jobs"]["package"]
    env = {**app_ci.get("env", {}), **job.get("env", {})}
    assert "secrets.WINDOWS_CERTIFICATE" in str(env)
    sign = [s for s in steps(job) if "signtool" in s.get("run", "")]
    assert sign, "no signtool step"
    for s in sign:
        assert "HAS_WINDOWS_SIGNING == 'true'" in s.get("if", ""), f"step {s.get('name')} is not gated"
        assert "runner.os == 'Windows'" in s.get("if", "")


def test_nfr_15d_package_publishes_a_sha256_file_for_each_artifact(app_ci):
    """[NFR-15d] The package job builds the NSIS installer on every event and writes a SHA-256 file for it."""
    job = app_ci["jobs"]["package"]
    text = runs(job)
    assert "pnpm tauri build --debug" in text
    for event in EVENTS:
        bundles = {e.get("os"): e.get("bundles") for e in matrix_entries(job, event)}
        assert bundles == {"windows-2025": "nsis"}, f"package on {event}: {bundles}"
    assert "sha256" in text.lower()
    uploads = [s for s in steps(job) if "actions/upload-artifact" in s.get("uses", "")]
    assert uploads and all(".sha256" in str(s.get("with", {}).get("path", "")) for s in uploads)


def test_nfr_15c_app_ci_makes_no_mac_app_store_build(app_ci):
    """[NFR-15c] No job builds a Mac App Store target."""
    assert not re.search(r"\bmas\b|app-store|appstore", all_runs(app_ci), re.IGNORECASE)


def test_sec_26_app_ci_has_an_adversarial_golden_set_hook(app_ci):
    """[SEC-26] A CI job runs the adversarial golden set tests (tagged SEC-26) on Windows and Linux."""
    job = app_ci["jobs"]["golden-set"]
    assert matrix_oses(job) == CI_OSES
    text = runs(job)
    assert "SEC-26" in text and "sec_26" in text


STRAWBERRY_PERL = r"C:\Strawberry\perl\bin\perl.exe"


def test_nfr_13a_windows_jobs_build_vendored_openssl_with_strawberry_perl(app_ci):
    """[NFR-13a] Windows jobs that compile Rust set OPENSSL_SRC_PERL to Strawberry Perl first (#375).

    The vendored OpenSSL in the SQLCipher build (#35) runs Configure with perl; Git Bash's MSYS
    perl lacks Locale::Maketext::Simple, so the build fails on windows runners without this.
    """
    for name, job in app_ci["jobs"].items():
        if not any(o and o.startswith("windows") for o in matrix_oses(job)):
            continue
        body = steps(job)
        builds = [i for i, s in enumerate(body) if re.search(r"\bcargo\b|tauri build", s.get("run", ""))]
        if not builds:
            continue
        sets = [i for i, s in enumerate(body)
                if "OPENSSL_SRC_PERL" in s.get("run", "") and STRAWBERRY_PERL in s.get("run", "")
                and "runner.os == 'Windows'" in str(s.get("if", ""))]
        assert sets, f"{name}: no Windows-only step sets OPENSSL_SRC_PERL to {STRAWBERRY_PERL}"
        assert sets[0] < builds[0], f"{name}: OPENSSL_SRC_PERL is set after the first cargo or tauri build step"
