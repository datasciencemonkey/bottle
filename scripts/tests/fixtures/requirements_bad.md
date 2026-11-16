# Broken fixture

## 5. Requirements tables

### 5.1 MB: Hub and app

| ID | Pri | Phase | Requirement | Acceptance criterion | Verify | Source | UC |
|---|---|---|---|---|---|---|---|
| MB-01 | M | MVP-D | The hub must create MOGs. | It works. | D · AT-01 | D1 | ALL |
| MB-02 | X | MVP-D | Bad priority. | It fails. | T · AT-01 | D2 | ALL |
| MB-03 | M | MVP-D | Too few cells. | T · AT-01 | D2 | ALL |
| MB-01 | M | MVP-D | Duplicate ID. | It fails. | T · AT-01 | D2 | ALL |

## 14. Traceability

### 14.3 Requirement-to-test summary

| Test | Requirements verified |
|---|---|
| AT-01 | MB-01, MB-99 |

## 15. Acceptance tests

| AT | Name | Procedure (STE) | Pass criterion | Gate |
|---|---|---|---|---|
| AT-01 | Create two MOGs | Create two MOGs. | Both survive. | device |
