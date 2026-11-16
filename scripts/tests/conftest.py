"""Shared helpers for the requirement-coverage script tests (#182)."""
import json
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"
REAL_REQUIREMENTS = ROOT / "research" / "final-requirements.md"

sys.path.insert(0, str(SCRIPTS))


def run_script(name, *args):
    """Run scripts/<name> with the current Python and return the completed process."""
    return subprocess.run([sys.executable, str(SCRIPTS / name), *map(str, args)],
                          cwd=ROOT, capture_output=True, text=True)


@pytest.fixture
def coverage(tmp_path):
    """Run req_coverage.py and return (process, markdown text, parsed JSON)."""
    def _run(requirements, *junit, extra=()):
        out, js = tmp_path / "cov.md", tmp_path / "cov.json"
        args = ["--requirements", requirements, "--out", out, "--json", js, *extra]
        if junit:
            args += ["--junit", *junit]
        proc = run_script("req_coverage.py", *args)
        md = out.read_text() if out.exists() else ""
        data = json.loads(js.read_text()) if js.exists() else {}
        return proc, md, data
    return _run
