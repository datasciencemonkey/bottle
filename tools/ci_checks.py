#!/usr/bin/env python3
"""Repository checks, run by CI on every pull request and push to main.

Each check prints PASS or FAIL. The script exits non-zero if any check fails.

  compile     every tools/*.py file compiles
  html        requirements.html is up to date with its sources
  mascot      mog-rive/scene.rml is up to date with tools/gen_mog_rml.py
  brand       the mascot has no Lava FF3621 fill and no Tail, Wisp or Brick node (UX-10a/b, AT-37)
  artifacts   no build output (build/, *.riv, *.rev) is committed
  structure   no junk, secrets, binaries or local tool state is committed; no file over 2 MB;
              every top-level entry is on the layout list; ADR numbers are unique; every
              crate is a Cargo workspace member. It also prints WARN lines: restructuring
              advice that does not fail the build.

Run:  uv run python tools/ci_checks.py [check ...]
The html and mascot checks regenerate the file; if it changes, commit the result.
"""
import glob
import os
import py_compile
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
HTML = os.path.join(ROOT, "requirements.html")
SCENE = os.path.join(ROOT, "mog-rive", "scene.rml")


def read(path):
    with open(path, "rb") as f:
        return f.read()


def regenerated(target, script):
    """Run a generator script and report whether its output file is unchanged."""
    before = read(target)
    proc = subprocess.run([sys.executable, os.path.join(ROOT, script)], cwd=ROOT,
                          capture_output=True, text=True)
    if proc.returncode != 0:
        return False, f"{script} failed:\n{proc.stdout}{proc.stderr}"
    if read(target) != before:
        return False, f"{os.path.relpath(target, ROOT)} is out of date: run {script} and commit the result"
    return True, f"{os.path.relpath(target, ROOT)} matches {script}"


def check_compile():
    bad = []
    for path in sorted(glob.glob(os.path.join(ROOT, "tools", "*.py"))):
        try:
            py_compile.compile(path, doraise=True)
        except py_compile.PyCompileError as e:
            bad.append(str(e))
    return (not bad), ("\n".join(bad) if bad else "all tools/*.py files compile")


def check_html():
    return regenerated(HTML, "tools/build_requirements_html.py")


def check_mascot():
    return regenerated(SCENE, "tools/gen_mog_rml.py")


def check_brand():
    text = read(SCENE).decode("utf-8")
    problems = []
    if re.search(r"FF3621", text, re.IGNORECASE):
        problems.append("scene.rml uses Lava FF3621 (UX-10b)")
    for name in ("Tail", "Wisp", "Brick"):
        if re.search(rf'name="{name}"', text):
            problems.append(f'scene.rml has a "{name}" node (UX-10a, AT-37)')
    return (not problems), ("\n".join(problems) if problems else "no FF3621, Tail, Wisp or Brick in scene.rml")


def check_artifacts():
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    bad = [f for f in files if re.search(r"(^|/)build/", f) or f.endswith((".riv", ".rev"))]
    return (not bad), ("committed build output: " + ", ".join(bad) if bad else "no build output is committed")


# ---------------------------------------------------------------- structure
# Tracked paths that never belong in the repository.
JUNK = [
    (r"(^|/)\.DS_Store$|(^|/)Thumbs\.db$|\.swp$|~$|\.orig$|\.bak$|\.tmp$", "OS or editor junk"),
    (r"\.log$", "a log file"),
    (r"(^|/)(node_modules|target|dist|build|__pycache__|\.venv|venv|reports|tmp|\.cache)/", "build or cache output"),
    (r"\.py[cod]$|\.riv$|\.rev$|\.(dmg|msi|msix|exe|app|dylib|so|dll|o|a|rlib|zip|tar|tgz|gz)$", "a compiled or packaged binary"),
    (r"(^|/)\.env(\.|$)|\.pem$|\.key$|\.p12$|\.pfx$|(^|/)\.databrickscfg$|(^|/)secrets/", "a secret or credential file"),
    (r"(^|/)\.(omc|isaac|idea|vscode)/|(^|/)\.claude/settings\.local\.json$", "local tool or editor state"),
]
MAX_BYTES = 2 * 1024 * 1024

# The agreed top-level layout (CONTRIBUTING.md "Layout"). A new entry needs a reason:
# add it here and to CONTRIBUTING.md in the same PR, or put it under an existing folder.
TOP_LEVEL = {
    # repo and tooling config
    ".github", ".config", ".gitignore", ".prettierignore", ".prettierrc.json",
    "eslint.config.js", "package.json", "pnpm-lock.yaml", "tsconfig.json", "vite.config.ts",
    "Cargo.toml", "Cargo.lock",
    # docs
    "AGENTS.md", "CONTRIBUTING.md", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md",
    "REQUIREMENTS.md", "requirements.html",
    # code
    "crates", "src", "src-tauri", "tests", "packages", "scripts", "tools", "spikes",
    # Vite page entries of the four window kinds (see WARN below)
    "brain.html", "card.html", "hub.html", "mog.html",
    # project content
    "docs", "research", "mog-rive",
}

# Known structural debt: printed as WARN on every run until it is fixed. Each entry is
# (condition on the tracked files, advice).
ADVICE = [
    (lambda fs: any(f.startswith("tools/") for f in fs) and any(f.startswith("scripts/") for f in fs),
     "Python helpers live in two folders, tools/ and scripts/. Move tools/*.py into scripts/ "
     "(update repo-checks.yml, AGENTS.md, README.md and the generator paths) so there is one place."),
    (lambda fs: sum(f in {"brain.html", "card.html", "hub.html", "mog.html"} for f in fs) > 0,
     "The four Vite page entries (brain/card/hub/mog.html) sit at the repo root. Move them under "
     "src/pages/ and point vite.config.ts build.rollupOptions.input and the window URLs in "
     "src-tauri/tauri.conf.json at the new paths (foundation lead owns these)."),
    (lambda fs: "REQUIREMENTS.md" in fs,
     "REQUIREMENTS.md is the superseded v0.2 draft. Move it to research/archive/ (or delete it) "
     "and update AGENTS.md and README.md, so the root holds only current documents."),
    (lambda fs: "requirements.html" in fs,
     "requirements.html is a 1.3 MB generated file at the root. Consider docs/requirements.html "
     "(and a path change in tools/build_requirements_html.py, AGENTS.md and README.md)."),
    (lambda fs: any(f.startswith("research/kimi-snapshot/") and f.endswith(".py") for f in fs),
     "research/kimi-snapshot/ holds runnable code that must never run. Move the snapshot to "
     "research/archive/kimi-snapshot/ so it is plainly reference data."),
    (lambda fs: any(f.startswith("packages/") for f in fs) and any(f.startswith("src/shared/") for f in fs),
     "Both packages/ and src/shared/ hold shared TypeScript. Pick one home for shared code and "
     "record the choice in CONTRIBUTING.md \"Layout\"."),
]


def tracked():
    return subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, text=True,
                          check=True).stdout.split("\0")[:-1]


def check_structure():
    files = tracked()
    problems = []
    for f in files:
        for pattern, what in JUNK:
            if re.search(pattern, f):
                problems.append(f"{f}: {what} is committed")
                break
    for f in files:
        path = os.path.join(ROOT, f)
        if os.path.isfile(path) and os.path.getsize(path) > MAX_BYTES:
            problems.append(f"{f}: {os.path.getsize(path) // 1024} KB is over the 2 MB limit; keep large files out of git")
    for top in sorted({f.split("/", 1)[0] for f in files} - TOP_LEVEL):
        problems.append(f"{top}: a new top-level entry; put it under an existing folder, or add it to "
                        f"TOP_LEVEL in tools/ci_checks.py and to CONTRIBUTING.md \"Layout\" with a reason")
    numbers = {}
    for f in files:
        if f.startswith("docs/adr/") and f.endswith(".md"):
            name = f.rsplit("/", 1)[1]
            m = re.match(r"^(\d{4})-[a-z0-9]+(-[a-z0-9]+)*\.md$", name)
            if not m:
                problems.append(f"{f}: ADR names must be NNNN-lower-case-slug.md")
                continue
            numbers.setdefault(m.group(1), []).append(name)
    for num, names in sorted(numbers.items()):
        if len(names) > 1:
            problems.append(f"docs/adr: ADR {num} is used {len(names)} times ({', '.join(names)}); renumber all but one")
    members = re.findall(r'"([^"]+)"', (re.search(r"members\s*=\s*\[([^\]]*)\]", read(os.path.join(ROOT, "Cargo.toml")).decode()) or [None, ""])[1])
    for crate in sorted({f.split("/")[1] for f in files if f.startswith("crates/") and f.endswith("/Cargo.toml")}):
        if f"crates/{crate}" not in members:
            problems.append(f"crates/{crate}: not a member of the Cargo workspace in Cargo.toml")
    for cond, advice in ADVICE:
        if cond(files):
            print(f"WARN  structure: {advice}")
    return (not problems), ("\n".join(problems) if problems else
                            f"{len(files)} tracked files: no junk, secrets or large files; layout and ADR numbers OK")


CHECKS = {"compile": check_compile, "html": check_html, "mascot": check_mascot,
          "brand": check_brand, "artifacts": check_artifacts,
          "structure": check_structure}


def main(names):
    names = names or list(CHECKS)
    unknown = [n for n in names if n not in CHECKS]
    if unknown:
        sys.exit(f"unknown check(s): {', '.join(unknown)}; choose from {', '.join(CHECKS)}")
    failed = 0
    for name in names:
        ok, detail = CHECKS[name]()
        print(f"{'PASS' if ok else 'FAIL'}  {name}: {detail}")
        failed += not ok
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
