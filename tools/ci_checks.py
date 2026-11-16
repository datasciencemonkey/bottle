#!/usr/bin/env python3
"""Repository checks, run by CI on every pull request and push to main.

Each check prints PASS or FAIL. The script exits non-zero if any check fails.

  compile     every tools/*.py file compiles
  html        requirements.html is up to date with its sources
  mascot      mog-rive/scene.rml is up to date with tools/gen_mog_rml.py
  brand       the mascot has no Lava FF3621 fill and no Tail, Wisp or Brick node (UX-10a/b, AT-37)
  artifacts   no build output (build/, *.riv, *.rev) is committed
  gates       no release gate is reported as a pass without evidence (AT-22, scripts/check_gate_evidence.py)
  brandgate   brand approval record and mascot art checks; pending approval is "not passed" and
              only blocks --external (SEC-22a/b, COL-104, scripts/check_brand_gate.py)
  names       no former name in UI text or metadata, the Labs label in the hub and About, no MOG in
              package or bundle IDs, and MOG only as a noun in UI text (MB-10a/b/c, AT-37, ADR 0030)

Run:  uv run python tools/ci_checks.py [check ...]
The html and mascot checks regenerate the file; if it changes, commit the result.
"""
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


def check_gates():
    proc = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "check_gate_evidence.py")],
                          cwd=ROOT, capture_output=True, text=True)
    return proc.returncode == 0, (proc.stdout + proc.stderr).strip()


def check_brandgate():
    proc = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "check_brand_gate.py")],
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
          "brandgate": check_brandgate,
          "names": check_names}


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
