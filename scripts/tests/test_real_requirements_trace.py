"""Regression guard for NFR-23a on the real requirements: every MUST row has an AT link in 14.3."""
import req_coverage
from conftest import REAL_REQUIREMENTS


def test_nfr_23a_every_must_row_has_an_at_link_in_14_3():
    data = req_coverage.analyse(str(REAL_REQUIREMENTS), [])
    assert data["must_without_at"] == []
