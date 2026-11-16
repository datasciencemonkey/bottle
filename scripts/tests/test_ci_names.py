"""Tests for the `names` check in tools/ci_checks.py (MB-10a, MB-10b, MB-10c, AT-37, #71)."""
import json
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
    (root / "src" / "views" / "hub").mkdir(parents=True)
    labs = "<p>Databricks Labs / experimental</p>"
    (root / "src" / "views" / "hub" / "Hub.tsx").write_text(
        f'export const Hub = () => <main aria-label="MOGs"><h1>Bottle</h1>{labs}</main>;\n')
    (root / "src" / "views" / "hub" / "About.tsx").write_text(
        f"export const About = () => <section><h2>About Bottle</h2>{labs}</section>;\n")
    (root / "src-tauri" / "tauri.conf.json").write_text(json.dumps(
        {"productName": "Bottle", "identifier": "com.databricks.bottle",
         "bundle": {"shortDescription": "Bottle (Databricks Labs / experimental)"}}))
    (root / "package.json").write_text(json.dumps({"name": "bottle", "description": "Bottle app"}))
    (root / "src-tauri" / "Cargo.toml").write_text('[package]\nname = "bottle"\n')
    return root


def test_mb_10a_names_check_passes_a_clean_tree(tmp_path):
    """[MB-10a] [AT-37] A tree that follows the rules has no problems."""
    assert ci_checks.naming_problems(good_tree(tmp_path)) == []


def test_mb_10a_names_check_finds_a_former_product_name(tmp_path):
    """[MB-10a] [AT-37] A former product name in UI text or metadata fails, in any spelling."""
    root = good_tree(tmp_path)
    (root / "docs" / "user" / "guide.md").write_text(f"Welcome to {OLD}.\n")
    (root / "package.json").write_text(
        json.dumps({"name": "bottle", "description": OLD.replace(" ", "").lower()}))
    problems = ci_checks.naming_problems(root)
    assert any("docs/user/guide.md" in p for p in problems)
    assert any("package.json" in p for p in problems)


def test_mb_10a_names_check_reads_html_entities(tmp_path):
    """[MB-10a] A former name split by an HTML space entity still fails."""
    root = good_tree(tmp_path)
    (root / "src" / "views" / "hub" / "Note.tsx").write_text(
        "export const N = () => <p>%s</p>;\n" % OLD.replace(" ", "&nbsp;"))
    assert any("Note.tsx" in p for p in ci_checks.naming_problems(root))


def test_mb_10a_names_check_skips_tests_and_records(tmp_path):
    """[MB-10a] Test files and agent records are not UI text."""
    root = good_tree(tmp_path)
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


def test_mb_10b_names_check_needs_the_labs_label_in_hub_and_about(tmp_path):
    """[MB-10b] [AT-37] The hub view and About show the Labs label."""
    root = good_tree(tmp_path)
    for name in ("Hub.tsx", "About.tsx"):
        (root / "src" / "views" / "hub" / name).write_text("export const X = () => <p>Bottle</p>;\n")
    problems = ci_checks.naming_problems(root)
    assert any("Hub.tsx" in p and "Labs" in p for p in problems)
    assert any("About.tsx" in p and "Labs" in p for p in problems)


def test_mb_10c_names_check_bans_mog_in_package_and_bundle_ids(tmp_path):
    """[MB-10c] [AT-37] The bundle ID, product name, binary, npm and app crate names have no MOG."""
    cases = [
        ("src-tauri/tauri.conf.json", {"productName": "Bottle", "identifier": "com.databricks.mog"}),
        ("src-tauri/tauri.conf.json", {"productName": "MOG", "identifier": "com.databricks.bottle"}),
        ("src-tauri/tauri.conf.json", {"productName": "Bottle", "identifier": "com.databricks.bottle",
                                       "mainBinaryName": "mog"}),
        ("package.json", {"name": "mog-app"}),
    ]
    for rel, value in cases:
        root = good_tree(tmp_path / str(len(rel)) / str(len(json.dumps(value))))
        (root / rel).write_text(json.dumps(value))
        assert any("MB-10c" in p for p in ci_checks.naming_problems(root)), value
    root = good_tree(tmp_path / "crate")
    (root / "src-tauri" / "Cargo.toml").write_text('[package]\nname = "mog-app"\n')
    assert any("Cargo.toml" in p and "MB-10c" in p for p in ci_checks.naming_problems(root))


def test_mb_10c_bundle_id_is_the_mdm_domain(tmp_path):
    """[MB-10c] B0: the bundle ID is com.databricks.bottle."""
    root = good_tree(tmp_path)
    (root / "src-tauri" / "tauri.conf.json").write_text(
        json.dumps({"productName": "Bottle", "identifier": "com.databricks.app"}))
    assert any("com.databricks.bottle" in p for p in ci_checks.naming_problems(root))


def test_mb_10c_names_check_flags_mog_in_view_text_and_descriptions(tmp_path):
    """[MB-10c] [AT-37] JSX text, a11y attributes and descriptions write MOG, not mog."""
    root = good_tree(tmp_path)
    (root / "src" / "views" / "hub" / "Add.tsx").write_text(
        'export const A = () => <button aria-label="New mog">Add a Mog</button>;\n')
    (root / "package.json").write_text(json.dumps({"name": "bottle", "description": "Bottle mogs"}))
    problems = ci_checks.naming_problems(root)
    assert any("Add.tsx" in p and "'mog'" in p for p in problems)
    assert any("Add.tsx" in p and "'Mog'" in p for p in problems)
    assert any("package.json" in p and "'mogs'" in p for p in problems)


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
