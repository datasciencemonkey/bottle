"""Tests for the app CI workflow and the pinned lockfiles (NFR-13a, NFR-15a/b, SEC-26, AT-37, #227).

Run:  uv run --no-project --with pytest --with pyyaml pytest scripts/tests -q
"""
import re

import pytest
import yaml
from conftest import ROOT

WORKFLOWS = ROOT / ".github" / "workflows"
APP_CI = WORKFLOWS / "app-ci.yml"
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


def matrix_oses(job):
    include = job.get("strategy", {}).get("matrix", {}).get("include", [])
    return {item.get("os") for item in include} | set(job.get("strategy", {}).get("matrix", {}).get("os", []))


def test_nfr_13a_app_ci_runs_on_macos_arm64_and_windows(app_ci):
    """[NFR-13a] The check, build and package jobs run on macOS (arm64) and on Windows."""
    for name in ("checks", "offline-build", "package"):
        oses = matrix_oses(app_ci["jobs"][name])
        assert "macos-15" in oses or "macos-14" in oses, f"{name}: no arm64 macOS runner in {oses}"
        assert any(o and o.startswith("windows") for o in oses), f"{name}: no Windows runner in {oses}"


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


def test_nfr_15a_macos_signing_and_notarization_skip_without_secrets(app_ci):
    """[NFR-15a] The macOS sign and notarize steps read secrets and run only when the secrets exist."""
    job = app_ci["jobs"]["package"]
    env = {**app_ci.get("env", {}), **job.get("env", {})}
    assert "secrets.APPLE_CERTIFICATE" in str(env)
    sign = [s for s in steps(job) if "notarytool" in s.get("run", "")]
    assert sign, "no notarization step"
    for s in sign:
        assert "HAS_APPLE_SIGNING == 'true'" in s.get("if", ""), f"step {s.get('name')} is not gated"
        assert "runner.os == 'macOS'" in s.get("if", "")


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
    """[NFR-15d] The package job builds the DMG and NSIS installers and writes a SHA-256 file for each."""
    job = app_ci["jobs"]["package"]
    text = runs(job)
    assert "pnpm tauri build --debug" in text
    bundles = {item.get("bundles") for item in job["strategy"]["matrix"]["include"]}
    assert {"dmg", "nsis"} <= bundles
    assert "sha256" in text.lower()
    uploads = [s for s in steps(job) if "actions/upload-artifact" in s.get("uses", "")]
    assert uploads and all(".sha256" in str(s.get("with", {}).get("path", "")) for s in uploads)


def test_nfr_15c_app_ci_makes_no_mac_app_store_build(app_ci):
    """[NFR-15c] No job builds a Mac App Store target."""
    assert not re.search(r"\bmas\b|app-store|appstore", all_runs(app_ci), re.IGNORECASE)


def test_sec_26_app_ci_has_an_adversarial_golden_set_hook(app_ci):
    """[SEC-26] A CI job runs the adversarial golden set tests (tagged SEC-26) on both OSes."""
    job = app_ci["jobs"]["golden-set"]
    assert len(matrix_oses(job)) >= 2
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
