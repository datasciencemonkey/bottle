"""Find the Rust and Node project roots that CI must build and test.

Rust root: a directory whose Cargo.toml declares [workspace], or a crate with no
enclosing [workspace]. Node root: a directory with package.json that is not a member
of an enclosing pnpm/npm workspace. Vendored, generated and untrusted folders are skipped.

Usage: python3 scripts/ci_discover.py [--github-output]   (#227)
"""
import json
import os
import sys

SKIP_DIRS = {".git", "node_modules", "target", "dist", "build", ".cache", "research", "mog-rive", "tmp", "gen"}


def _walk(root, filename):
    root = str(root)
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS and not d.startswith("."))
        if filename in filenames:
            yield os.path.relpath(dirpath, root)


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def _ancestors(rel):
    parts = [] if rel == "." else rel.split(os.sep)
    for i in range(len(parts) - 1, -1, -1):
        yield os.sep.join(parts[:i]) or "."


def rust_roots(root):
    root = str(root)
    manifests = list(_walk(root, "Cargo.toml"))
    has_ws = {d for d in manifests if "[workspace]" in _read(os.path.join(root, d, "Cargo.toml"))}
    out = []
    for d in manifests:
        if d in has_ws or not any(a in has_ws for a in _ancestors(d)):
            out.append(d)
    return sorted(out, key=lambda p: (p != ".", p))


def _is_node_workspace(root, d):
    if os.path.exists(os.path.join(root, d, "pnpm-workspace.yaml")):
        return True
    try:
        return "workspaces" in json.loads(_read(os.path.join(root, d, "package.json")))
    except ValueError:
        return False


def node_roots(root):
    root = str(root)
    pkgs = list(_walk(root, "package.json"))
    ws = {d for d in pkgs if _is_node_workspace(root, d)}
    out = [d for d in pkgs if d in ws or not any(a in ws for a in _ancestors(d))]
    return sorted(out, key=lambda p: (p != ".", p))


def main(argv):
    root = os.getcwd()
    data = {"rust": rust_roots(root), "node": node_roots(root)}
    if "--github-output" in argv and os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as f:
            for k, v in data.items():
                f.write(f"{k}={json.dumps(v)}\n")
    print(json.dumps(data))


if __name__ == "__main__":
    main(sys.argv[1:])
