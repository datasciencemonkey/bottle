"""Tests for scripts/req_coverage.py and the shared parser (NFR-23a, AT-22, #182)."""
from conftest import FIXTURES, REAL_REQUIREMENTS

SMALL = FIXTURES / "requirements_small.md"
BAD = FIXTURES / "requirements_bad.md"
MIXED = FIXTURES / "junit_mixed.xml"
UNKNOWN = FIXTURES / "junit_unknown.xml"


def test_nfr_23a_parser_reads_requirement_rows_with_escaped_pipes():
    """[NFR-23a] Section 5 rows parse into ID, priority and phase; `\\|` stays inside a cell."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert doc.errors == []
    assert set(doc.requirements) == {"MB-01", "MB-02", "CAP-14a", "CAP-14b", "MEM-01", "MEM-01b",
                                     "NFR-23a", "W-07d"}
    mb02 = doc.requirements["MB-02"]
    assert (mb02.priority, mb02.phase) == ("M", "MVP-D")
    assert "a\\|b" in mb02.text
    assert doc.requirements["CAP-14b"].phase == "MVP-M"


def test_nfr_23a_parser_knows_at_arc_and_spike_ids():
    """[NFR-23a] The known-ID set holds requirement rows, ATs and other row IDs such as ARC-1."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert set(doc.acceptance_tests) == {"AT-01", "AT-02", "AT-22", "AT-31"}
    for known in ("ARC-1", "ARC-F1", "SP-01", "AT-22", "NFR-23a"):
        assert known in doc.known_ids


def test_nfr_23a_parser_maps_14_3_and_verify_links():
    """[NFR-23a] AT links come from the 14.3 table; the Verify column is kept separately."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert doc.at_links_14_3["CAP-14b"] == {"AT-01"}
    assert "W-07d" not in doc.at_links_14_3
    assert doc.requirements["W-07d"].verify_ats == {"AT-02"}


def test_nfr_23a_bracket_tags_extract_several_ids():
    """[NFR-23a] `[MB-02, AT-02]` gives both IDs; `[slow]` is not a tag."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    known, unknown = req_parse.extract_ids("[MB-02, AT-02] rejects a key [slow]", doc.known_ids)
    assert known == {"MB-02", "AT-02"} and unknown == set()


def test_nfr_23a_snake_names_match_case_insensitively():
    """[NFR-23a] `cap_14b_stops_on_esc` maps to CAP-14b; `test_nfr_23a_x` maps to NFR-23a."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("cap_14b_stops_on_esc", doc.known_ids) == ({"CAP-14b"}, set())
    assert req_parse.extract_ids("test_nfr_23a_flags", doc.known_ids) == ({"NFR-23a"}, set())
    assert req_parse.extract_ids("mog_core::tests::arc_f1_restarts", doc.known_ids) == ({"ARC-F1"}, set())


def test_nfr_23a_snake_names_prefer_longest_match():
    """[NFR-23a] `mem_01_b_x` is MEM-01b (longest known ID), not MEM-01."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("mem_01_b_compresses", doc.known_ids) == ({"MEM-01b"}, set())
    assert req_parse.extract_ids("mem_01_syncs", doc.known_ids) == ({"MEM-01"}, set())


def test_nfr_23a_snake_words_without_known_prefix_are_ignored():
    """[NFR-23a] Words like `retries_3_times` are not IDs."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("retries_3_times", doc.known_ids) == (set(), set())


def test_nfr_23a_unknown_ids_exit_1(coverage):
    """[NFR-23a] A test that names an ID that is not in the requirements gives exit 1 and is listed."""
    proc, md, data = coverage(SMALL, UNKNOWN)
    assert proc.returncode == 1, proc.stdout + proc.stderr
    assert sorted(data["unknown_ids"]) == ["CAP-99", "MB-77"]
    assert "CAP-99" in md and "MB-77" in md
    assert "CAP-99" in proc.stdout + proc.stderr


def test_nfr_23a_known_ids_only_exit_0(coverage):
    """[NFR-23a] With only known IDs the exit code is 0 and the passing IDs are counted."""
    proc, md, data = coverage(SMALL, MIXED)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert data["unknown_ids"] == []
    # cap_14a failed and mem_01_b was skipped, so they do not count as passing.
    assert sorted(data["passing_ids"]) == ["AT-02", "CAP-14b", "MB-01", "MB-02", "NFR-23a"]
    assert data["requirements_total"] == 8
    assert data["requirements_with_passing_test"] == 4


def test_nfr_23a_missing_junit_is_reported_not_fatal(coverage, tmp_path):
    """[NFR-23a] A missing JUnit file is listed in the report and does not change the exit code."""
    missing = tmp_path / "nope.xml"
    proc, md, data = coverage(SMALL, MIXED, missing)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert data["missing_junit"] == [str(missing)]
    assert "nope.xml" in md


def test_nfr_23a_broken_junit_is_an_error(coverage):
    """[NFR-23a] An unreadable JUnit file is a parse error and gives exit 1."""
    proc, md, data = coverage(SMALL, FIXTURES / "junit_broken.xml")
    assert proc.returncode == 1
    assert any("junit_broken.xml" in e for e in data["parse_errors"])


def test_nfr_23a_flags_must_row_without_at(coverage):
    """[NFR-23a] A MUST row missing from the 14.3 AT table is reported; S and C rows are not."""
    proc, md, data = coverage(SMALL)
    assert proc.returncode == 0
    assert data["must_without_at"] == ["W-07d"]
    assert "W-07d" in md
    assert "MEM-01" not in data["must_without_at"]


def test_nfr_23a_strict_must_makes_missing_link_fatal(coverage):
    """[NFR-23a] With --strict-must a MUST row without an AT link gives exit 1."""
    proc, md, data = coverage(SMALL, extra=["--strict-must"])
    assert proc.returncode == 1
    assert data["must_without_at"] == ["W-07d"]


def test_nfr_23a_report_has_totals_and_milestones(coverage):
    """[NFR-23a] The markdown report has totals and one row per milestone."""
    proc, md, data = coverage(SMALL, MIXED)
    assert "# Requirements coverage" in md
    for phase in ("MVP-D", "MVP-M", "v1"):
        assert f"| {phase} |" in md
    assert data["per_phase"]["MVP-D"] == {"rows": 5, "must": 5, "with_passing_test": 3}
    assert data["per_phase"]["MVP-M"] == {"rows": 1, "must": 1, "with_passing_test": 1}


def test_nfr_23a_bad_requirements_give_parse_errors(coverage):
    """[NFR-23a] A bad priority, a short row, a duplicate ID and an undefined 14.3 ID are parse errors."""
    proc, md, data = coverage(BAD)
    assert proc.returncode == 1
    errors = "\n".join(data["parse_errors"])
    for needle in ("MB-02", "MB-03", "MB-01", "MB-99"):
        assert needle in errors
    assert "Parse errors" in md


def test_nfr_23a_real_requirements_parse_with_zero_errors(coverage):
    """[NFR-23a] [AT-22] The real requirements file parses with no errors."""
    proc, md, data = coverage(REAL_REQUIREMENTS)
    assert data["parse_errors"] == [], data["parse_errors"]
    assert data["requirements_total"] > 700
    assert data["acceptance_tests_total"] == 43
    assert "NFR-23a" in data["must_ids"] and "NFR-23b" in data["must_ids"]
    assert proc.returncode == 0, proc.stdout + proc.stderr


def test_nfr_23a_real_requirements_must_rows_without_at_are_must_rows(coverage):
    """[NFR-23a] Every row reported without an AT link on the real file is a MUST row."""
    proc, md, data = coverage(REAL_REQUIREMENTS)
    assert set(data["must_without_at"]) <= set(data["must_ids"])
    assert "MUST rows without an AT link" in md


def test_nfr_23a_pytest_param_brackets_are_not_ids():
    """[NFR-23a] pytest parameter IDs like `[case-1]`, `[a-2]` and `[H-264]` are not requirement tags."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    for name in ("test_x[case-1]", "test_y[a-2]", "test_array_1_index[param-1]", "renders [ok] and [H-264]",
                 "test_z[mb-01]"):
        assert req_parse.extract_ids(name, doc.known_ids) == (set(), set()), name


def test_nfr_23a_bracket_with_known_prefix_and_unknown_id_is_unknown():
    """[NFR-23a] `[CAP-999]` and `[MB-2]` use a real prefix with a missing ID, so they stay unknown."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("[CAP-999] x", doc.known_ids) == (set(), {"CAP-999"})
    assert req_parse.extract_ids("[MB-2] x", doc.known_ids) == (set(), {"MB-2"})


def test_nfr_23a_short_snake_numbers_without_match_are_ignored():
    """[NFR-23a] `test_w_2` and `test_g_1_case` are not IDs: real IDs have 2 digits, except ARC rows."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("test_w_2", doc.known_ids) == (set(), set())
    assert req_parse.extract_ids("test_mb_1_case", doc.known_ids) == (set(), set())
    assert req_parse.extract_ids("arc_1_one_process", doc.known_ids) == ({"ARC-1"}, set())


def test_nfr_23a_unknown_snake_id_drops_the_next_word():
    """[NFR-23a] `mb_77_x` is reported as MB-77, not MB-77x."""
    import req_parse
    doc = req_parse.parse(SMALL.read_text())
    assert req_parse.extract_ids("mb_77_x", doc.known_ids) == (set(), {"MB-77"})


def test_nfr_23a_real_file_must_rows_without_at_match_independent_count(coverage):
    """[NFR-23a] The MUST rows without a 14.3 link equal a separate count from the raw tables."""
    import re
    src = REAL_REQUIREMENTS.read_text()
    s143 = src[src.index("### 14.3"):src.index("## 15. Acceptance tests")]
    linked = {i.strip() for line in s143.splitlines() if line.startswith("| AT-")
              for i in line.split("|")[2].split(",")}
    sec5 = src[src.index("## 5. Requirements tables"):src.index("## 6. ")]
    must = [m.group(1) for m in re.finditer(r"^\| ([A-Z]+-\d+[a-z]?) \| M \|", sec5, re.M)]
    proc, md, data = coverage(REAL_REQUIREMENTS)
    assert data["must_without_at"] == [r for r in must if r not in linked]
    assert len(data["must_ids"]) == len(must)
