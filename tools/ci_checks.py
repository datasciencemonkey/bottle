#!/usr/bin/env python3
"""Repository checks, run by CI on every pull request and push to main.

Each check prints PASS or FAIL. The script exits non-zero if any check fails.

  compile     every tools/*.py file compiles
  html        requirements.html is up to date with its sources
  mascot      mog-rive/scene.rml is up to date with tools/gen_mog_rml.py
  brand       the mascot has no Lava FF3621 fill and no Tail, Wisp or Brick node (UX-10a/b, AT-37)
  artifacts   no build output (build/, *.riv, *.rev) is committed

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


CHECKS = {"compile": check_compile, "html": check_html, "mascot": check_mascot,
          "brand": check_brand, "artifacts": check_artifacts}


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
