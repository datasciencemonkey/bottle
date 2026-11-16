#!/usr/bin/env python3
"""Repository checks, run by CI on every pull request and push to main.

Each check prints PASS or FAIL. The script exits non-zero if any check fails.

  compile     every tools/*.py file compiles
  html        requirements.html is up to date with its sources
  mascot      mog-rive/scene.rml is up to date with tools/gen_mog_rml.py
  brand       the mascot has no Lava FF3621 fill and no Tail, Wisp or Brick node (UX-10a/b, AT-37)
  artifacts   no build output (build/, *.riv, *.rev) is committed
  gates       no release gate is reported as a pass without evidence (AT-22, scripts/check_gate_evidence.py)
  names       no former name in UI text or metadata, the Labs label in the hub and About, no MOG in
              package or bundle IDs, and MOG only as a noun in UI text (MB-10a/b/c, AT-37, ADR 0030)
  structure   no junk, secrets, binaries or local tool state is committed; no file over 2 MB;
              every top-level entry is on the layout list; ADR numbers are unique; every
              crate is a Cargo workspace member. It also prints WARN lines: restructuring
              advice that does not fail the build.

Run:  uv run python tools/ci_checks.py [check ...]
The html and mascot checks regenerate the file; if it changes, commit the result.
"""
import fnmatch
import glob
import json
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
    # Build and cache folders, only at the repo root or the root of a crate, package or spike,
    # so a legitimate source folder such as src/build/ is not flagged.
    (r"^((crates|packages|spikes)/[^/]+/|src-tauri/)?(node_modules|target|dist|build|\.venv|venv|reports|tmp|\.cache)/", "build or cache output"),
    (r"(^|/)__pycache__/", "build or cache output"),
    (r"\.py[cod]$|\.riv$|\.rev$|\.(dmg|msi|msix|exe|app|dylib|so|dll|o|a|rlib|zip|tar|tgz|gz)$", "a compiled or packaged binary"),
    (r"(^|/)\.env(\.(?!example$|sample$|template$)[^/]*)?$|\.pem$|\.key$|\.p12$|\.pfx$|(^|/)\.databrickscfg$|(^|/)secrets/",
     "a secret or credential file (.env.example, .env.sample and .env.template are allowed)"),
    (r"(^|/)\.(omc|isaac|idea|vscode)/|(^|/)\.claude/settings\.local\.json$", "local tool or editor state"),
]
MAX_BYTES = 2 * 1024 * 1024

# The agreed top-level layout (CONTRIBUTING.md "Layout"). A new entry needs a reason:
# add it here and to CONTRIBUTING.md in the same PR, or put it under an existing folder.
TOP_LEVEL = {
    # repo and tooling config
    ".github", ".config", ".cargo", ".gitignore", ".gitattributes", ".prettierignore", ".prettierrc.json",
    ".editorconfig", ".nvmrc", ".node-version", ".npmrc", "rust-toolchain.toml",
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
     "src/pages/ and update vite.config.ts (build.rollupOptions.input), the window URLs in "
     "src-tauri/src/windows.rs and the paths in src-tauri/tests/skeleton.rs (foundation lead owns these)."),
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


def workspace_members():
    """The `members` and `exclude` lists of [workspace] in Cargo.toml (globs allowed)."""
    text = read(os.path.join(ROOT, "Cargo.toml")).decode()
    text = "\n".join(line.split("#", 1)[0] for line in text.splitlines())   # drop comments
    ws = re.search(r"^\[workspace\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    body = ws.group(1) if ws else ""

    def array(key):
        m = re.search(rf"^\s*{key}\s*=\s*\[(.*?)\]", body, re.M | re.S)
        return re.findall(r'"([^"]+)"', m.group(1)) if m else []
    return array("members"), array("exclude")


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
                        f"TOP_LEVEL in tools/ci_checks.py with a reason and describe it in CONTRIBUTING.md \"Layout\"")
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
    members, excluded = workspace_members()
    for crate in sorted({f.split("/")[1] for f in files if f.startswith("crates/") and f.endswith("/Cargo.toml")}):
        path = f"crates/{crate}"
        if not any(fnmatch.fnmatch(path, m) for m in members) or any(fnmatch.fnmatch(path, x) for x in excluded):
            problems.append(f"{path}: not a member of the Cargo workspace in Cargo.toml")
    for cond, advice in ADVICE:
        if cond(files):
            print(f"WARN  structure: {advice}")
    return (not problems), ("\n".join(problems) if problems else
                            f"{len(files)} tracked files: no junk, secrets or large files; layout and ADR numbers OK")


def check_gates():
    proc = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "check_gate_evidence.py")],
                          cwd=ROOT, capture_output=True, text=True)
    return proc.returncode == 0, (proc.stdout + proc.stderr).strip()


# ---------- names: MB-10a, MB-10b, MB-10c, AT-37 (#71, ADR 0030) ----------
# This check runs in the repo-checks workflow today. The pnpm test
# `src/acceptance/at-37-naming.test.tsx` holds the same rules and also renders the views.
# The patterns are built from parts, so this file holds no banned name.
LABS_LABEL = "Databricks Labs / experimental"
LABS_TITLE = f"Bottle ({LABS_LABEL})"
BUNDLE_ID = "com.databricks.bottle"
SEP = r"(?:[\s_-]|&nbsp;|&#160;|&#x0*a0;|&#32;)*"
BANNED_NAME = re.compile("mo" + "ther" + SEP + "bri" + "cks", re.IGNORECASE)
JSX_TEXT = re.compile(r">([^<>{}()=;]+)[<{]")
UI_ATTR = re.compile(r'\b(?:aria-label|title|alt|placeholder)="([^"]*)"')
LABS_VIEWS = ("src/views/hub/Hub.tsx", "src/views/hub/About.tsx")
MOG_NOUN = re.compile(r"(?<![\w./:<>-])mogs?(?![\w/:<>-]|\.\w)", re.IGNORECASE)
VIEW_PAGES = ("hub.html", "mog.html", "card.html", "brain.html")
NAME_FILES = VIEW_PAGES + ("package.json", "Cargo.toml", "THIRD_PARTY_NOTICES.md",
                           "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml")
NAME_DIRS = ("src", "src-tauri/src", "src-tauri/capabilities", "src-tauri/permissions", "docs/user")
SKIP_DIRS = {"node_modules", "target", "dist", "icons", "gen"}


def naming_files(root):
    """Repository-relative paths of UI text and package metadata (ADR 0030)."""
    found = [f for f in NAME_FILES if os.path.isfile(os.path.join(root, f))]
    crates = os.path.join(root, "crates")
    dirs = list(NAME_DIRS)
    if os.path.isdir(crates):
        for c in sorted(os.listdir(crates)):
            found += [f"crates/{c}/Cargo.toml"] if os.path.isfile(os.path.join(crates, c, "Cargo.toml")) else []
            dirs.append(f"crates/{c}/src")
    for d in dirs:
        for dirpath, dirnames, filenames in os.walk(os.path.join(root, d)):
            dirnames[:] = sorted(n for n in dirnames if n not in SKIP_DIRS)
            for name in sorted(filenames):
                if re.search(r"\.test\.tsx?$|\.(png|icns|ico|riv|wasm)$", name):
                    continue
                found.append(os.path.relpath(os.path.join(dirpath, name), root).replace(os.sep, "/"))
    return found


def mog_noun_hits(text):
    return [w for w in MOG_NOUN.findall(text) if w not in ("MOG", "MOGs")]


def naming_problems(root):
    problems = []
    texts = {}
    for rel in naming_files(root):
        with open(os.path.join(root, rel), encoding="utf-8", errors="replace") as f:
            texts[rel] = f.read()
        for hit in BANNED_NAME.findall(texts[rel]):
            problems.append(f"{rel}: former product name {hit!r} (MB-10a)")
    titles = {}
    for rel in VIEW_PAGES:
        m = re.search(r"<title>(.*?)</title>", texts.get(rel, ""), re.IGNORECASE | re.DOTALL)
        if m:
            titles[rel] = m.group(1).strip()
    if titles.get("hub.html") != LABS_TITLE:
        problems.append(f"hub.html: the page title must be {LABS_TITLE!r} (Labs label, MB-10b)")
    windows = texts.get("src-tauri/src/windows.rs", "")
    window_titles = re.findall(r'\btitle:\s*"([^"]*)"', windows)
    if LABS_TITLE not in window_titles:
        problems.append(f"src-tauri/src/windows.rs: the hub title must be {LABS_TITLE!r} (Labs label, MB-10b)")
    for rel in LABS_VIEWS:
        if LABS_LABEL not in texts.get(rel, ""):
            problems.append(f"{rel}: the view must show {LABS_LABEL!r} (Labs label, MB-10b)")
    problems += id_problems(texts)
    nouns = [(f"{rel} <title>", t) for rel, t in titles.items()]
    nouns += [(rel, t) for rel, t in descriptions(texts)]
    for rel, text in texts.items():
        if rel.startswith("src/views/") and rel.endswith(".tsx"):
            nouns += [(rel, t) for t in JSX_TEXT.findall(text) + UI_ATTR.findall(text)]
    nouns += [("src-tauri/src/windows.rs title", t) for t in window_titles]
    for rel, text in texts.items():
        if rel.startswith("docs/user/") and rel.endswith(".md"):
            prose = re.sub(r"^(```|~~~).*?^\1", "", text, flags=re.MULTILINE | re.DOTALL)
            nouns.append((rel, re.sub(r"`[^`\n]*`", "", prose)))
    for where, text in nouns:
        for hit in mog_noun_hits(text):
            problems.append(f"{where}: write MOG, not {hit!r} (MB-10c)")
    return problems


def load_json(texts, rel):
    try:
        return json.loads(texts.get(rel, "{}"))
    except ValueError:
        return {}


def descriptions(texts):
    """The bundle and npm descriptions: user-visible package metadata."""
    conf = load_json(texts, "src-tauri/tauri.conf.json")
    bundle = conf.get("bundle", {}) if isinstance(conf, dict) else {}
    out = [("src-tauri/tauri.conf.json", str(bundle.get(k, ""))) for k in ("shortDescription", "longDescription")]
    package = load_json(texts, "package.json")
    out.append(("package.json", str(package.get("description", "")) if isinstance(package, dict) else ""))
    return out


def id_problems(texts):
    """MB-10c and B0: no MOG in the bundle ID, product name, binary, npm package or app crate (ADR 0002)."""
    problems = []
    conf = load_json(texts, "src-tauri/tauri.conf.json")
    package = load_json(texts, "package.json")
    if conf.get("identifier") != BUNDLE_ID:
        problems.append(f"src-tauri/tauri.conf.json: the bundle ID must be {BUNDLE_ID!r} (B0, MB-10c)")
    ids = [("src-tauri/tauri.conf.json", conf.get(k)) for k in ("identifier", "productName", "mainBinaryName")]
    ids.append(("package.json", package.get("name")))
    crate = re.search(r'^name\s*=\s*"([^"]+)"', texts.get("src-tauri/Cargo.toml", ""), re.MULTILINE)
    ids.append(("src-tauri/Cargo.toml", crate.group(1) if crate else None))
    for rel, value in ids:
        if isinstance(value, str) and "mog" in value.lower():
            problems.append(f"{rel}: the ID {value!r} contains MOG (MB-10c)")
    return problems


def check_names():
    problems = naming_problems(ROOT)
    return (not problems), ("\n".join(problems) if problems
                            else "former names, Labs label, IDs and the MOG noun hold in UI text and metadata")


CHECKS = {"compile": check_compile, "html": check_html, "mascot": check_mascot,
          "brand": check_brand, "artifacts": check_artifacts, "gates": check_gates,
          "names": check_names, "structure": check_structure}


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
