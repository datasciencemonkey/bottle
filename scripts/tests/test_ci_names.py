"""Tests for the `names` check in tools/ci_checks.py (MB-10a, MB-10b, MB-10c, AT-37, #71)."""
import subprocess
import sys

from conftest import ROOT

sys.path.insert(0, str(ROOT / "tools"))
import ci_checks  # noqa: E402

OLD = "Mother" + " " + "Bricks"
HUB_TITLE = "Bottle (Databricks Labs / experimental)"


def page(title):
    return f"<html><head><title>{title}</title></head><body></body></html>\n"


def good_tree(root):
    """A minimal repository that follows every naming rule."""
    (root / "src-tauri" / "src").mkdir(parents=True)
    (root / "docs" / "user").mkdir(parents=True)
    (root / "hub.html").write_text(page(HUB_TITLE))
    (root / "mog.html").write_text(page("Bottle MOG"))
    (root / "card.html").write_text(page("Bottle MOG card"))
    (root / "brain.html").write_text(page("Bottle brain"))
    (root / "src-tauri" / "src" / "windows.rs").write_text(
        f'a {{ title: "{HUB_TITLE}" }}\nb {{ title: "MOG card", url: "mog.html" }}\n')
    (root / "docs" / "user" / "guide.md").write_text("A MOG floats. See `mog-core`.\n")
    return root


def test_mb_10a_names_check_passes_a_clean_tree(tmp_path):
    """[MB-10a] [AT-37] A tree that follows the rules has no problems."""
    assert ci_checks.naming_problems(good_tree(tmp_path)) == []


def test_mb_10a_names_check_finds_a_former_product_name(tmp_path):
    """[MB-10a] [AT-37] A former product name in UI text or metadata fails, in any spelling."""
    root = good_tree(tmp_path)
    (root / "docs" / "user" / "guide.md").write_text(f"Welcome to {OLD}.\n")
    (root / "package.json").write_text('{"description": "%s"}\n' % OLD.replace(" ", "").lower())
    problems = ci_checks.naming_problems(root)
    assert any("docs/user/guide.md" in p for p in problems)
    assert any("package.json" in p for p in problems)


def test_mb_10a_names_check_skips_tests_and_records(tmp_path):
    """[MB-10a] Test files and agent records are not UI text."""
    root = good_tree(tmp_path)
    (root / "src").mkdir()
    (root / "src" / "x.test.ts").write_text(f"// {OLD}\n")
    (root / "research").mkdir()
    (root / "research" / "notes.md").write_text(f"formerly {OLD}\n")
    assert ci_checks.naming_problems(root) == []


def test_mb_10b_names_check_needs_the_labs_label_in_the_hub_title(tmp_path):
    """[MB-10b] [AT-37] The hub page title and the native hub title show the Labs label."""
    root = good_tree(tmp_path)
    (root / "hub.html").write_text(page("Bottle hub"))
    (root / "src-tauri" / "src" / "windows.rs").write_text('a { title: "Bottle" }\n')
    problems = ci_checks.naming_problems(root)
    assert any("hub.html" in p and "Labs" in p for p in problems)
    assert any("windows.rs" in p and "Labs" in p for p in problems)


def test_mb_10c_names_check_flags_mog_not_written_as_the_noun(tmp_path):
    """[MB-10c] [AT-37] Titles and user docs write MOG, not mog or Mog; code names are skipped."""
    root = good_tree(tmp_path)
    (root / "mog.html").write_text(page("Bottle mog"))
    (root / "docs" / "user" / "guide.md").write_text("Add a Mog. Code: `mog-core`, mog-<id>.\n")
    problems = ci_checks.naming_problems(root)
    assert any("mog.html" in p for p in problems)
    assert any("guide.md" in p and "'Mog'" in p for p in problems)
    assert len(problems) == 2


def test_mb_10a_names_check_passes_on_this_repository():
    """[MB-10a] [MB-10b] [MB-10c] [AT-37] The real repository passes `ci_checks.py names`."""
    proc = subprocess.run([sys.executable, str(ROOT / "tools" / "ci_checks.py"), "names"],
                          cwd=ROOT, capture_output=True, text=True)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert proc.stdout.startswith("PASS  names")
