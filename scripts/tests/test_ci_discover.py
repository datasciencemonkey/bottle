"""Tests for scripts/ci_discover.py (#227): find the Rust and Node roots for the CI matrix."""
import ci_discover
from conftest import ROOT


def write(root, rel, text=""):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def test_standalone_crate_is_a_rust_root(tmp_path):
    write(tmp_path, "crates/cli-runner/Cargo.toml", '[package]\nname="cli-runner"\n\n[workspace]\n')
    assert ci_discover.rust_roots(tmp_path) == ["crates/cli-runner"]


def test_workspace_member_is_not_its_own_root(tmp_path):
    write(tmp_path, "Cargo.toml", '[workspace]\nmembers=["crates/mog-core"]\n')
    write(tmp_path, "crates/mog-core/Cargo.toml", '[package]\nname="mog-core"\n')
    assert ci_discover.rust_roots(tmp_path) == ["."]


def test_nested_crate_with_own_workspace_is_separate_root(tmp_path):
    write(tmp_path, "Cargo.toml", '[workspace]\nmembers=["crates/mog-core"]\n')
    write(tmp_path, "crates/mog-core/Cargo.toml", '[package]\nname="mog-core"\n')
    write(tmp_path, "crates/http-policy/Cargo.toml", '[package]\nname="http-policy"\n[workspace]\n')
    assert ci_discover.rust_roots(tmp_path) == [".", "crates/http-policy"]


def test_target_and_node_modules_are_ignored(tmp_path):
    write(tmp_path, "crates/a/Cargo.toml", "[package]\n[workspace]\n")
    write(tmp_path, "crates/a/target/x/Cargo.toml", "[package]\n[workspace]\n")
    write(tmp_path, "packages/p/package.json", "{}")
    write(tmp_path, "packages/p/node_modules/dep/package.json", "{}")
    assert ci_discover.rust_roots(tmp_path) == ["crates/a"]
    assert ci_discover.node_roots(tmp_path) == ["packages/p"]


def test_node_workspace_members_are_not_separate_roots(tmp_path):
    write(tmp_path, "package.json", '{"name":"bottle"}')
    write(tmp_path, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n")
    write(tmp_path, "packages/genie-sanitize/package.json", '{"name":"x"}')
    assert ci_discover.node_roots(tmp_path) == ["."]


def test_standalone_node_package_without_root_workspace(tmp_path):
    write(tmp_path, "package.json", '{"name":"bottle"}')
    write(tmp_path, "packages/genie-sanitize/package.json", '{"name":"x"}')
    assert ci_discover.node_roots(tmp_path) == [".", "packages/genie-sanitize"]


def test_research_and_untrusted_dirs_are_ignored(tmp_path):
    write(tmp_path, "research/kimi-snapshot/tools/package.json", "{}")
    write(tmp_path, "mog-rive/package.json", "{}")
    assert ci_discover.node_roots(tmp_path) == []


def test_real_repo_has_root_projects():
    assert ci_discover.rust_roots(ROOT)[0] == "."
    assert ci_discover.node_roots(ROOT)[0] == "."
    assert not any(p.startswith(("research", "mog-rive", "src-tauri")) for p in ci_discover.rust_roots(ROOT))
