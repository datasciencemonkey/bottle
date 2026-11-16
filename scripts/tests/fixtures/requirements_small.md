# Fixture requirements

## 4. Architecture

### 4.2 Process model

| ID | Rule |
|---|---|
| ARC-1 | One OS process. |

### 6.7 Failure model

| ID | Failure | Handling |
|---|---|---|
| ARC-F1 | The brain crashes. | Restart it. |

## 5. Requirements tables

### 5.1 MB: Hub and app

| ID | Pri | Phase | Requirement | Acceptance criterion | Verify | Source | UC |
|---|---|---|---|---|---|---|---|
| MB-01 | M | MVP-D | The hub must create MOGs. | It works. | D · AT-01 | D1 | ALL |
| MB-02 | M | MVP-D | The hub must reject `a\|b` keys. | It rejects them. | T · AT-02 | D2 | ALL |

### 5.3 CAP: Capture

| ID | Pri | Phase | Requirement | Acceptance criterion | Verify | Source | UC |
|---|---|---|---|---|---|---|---|
| CAP-14a | M | MVP-D | Esc must stop capture. | It stops. | T · AT-01 | D3 | UC1 |
| CAP-14b | M | MVP-M | Esc must stop hands-free capture. | It stops. | T · AT-01 | D3 | MTG |
| MEM-01 | S | v1 | Memory should sync. | It syncs. | T · AT-02 | D4 | UC2 |
| MEM-01b | C | v1 | Memory could compress. | It compresses. | T · AT-02 | D4 | UC2 |

### 5.11 NFR: Performance

| ID | Pri | Phase | Requirement | Acceptance criterion | Verify | Source | UC |
|---|---|---|---|---|---|---|---|
| NFR-23a | M | MVP-D | Each MUST requirement must link to a test. | 14.3 shows no MUST row without a test. | A · AT-22 | D79 | ALL |
| W-07d | M | MVP-D | The CLI check must run. | It runs. | T · AT-02 | D5 | ALL |

## 14. Traceability

### 14.3 Requirement-to-test summary

| Test | Requirements verified |
|---|---|
| AT-01 | MB-01, CAP-14a, CAP-14b |
| AT-02 | MB-02, MEM-01, MEM-01b |
| AT-22 | NFR-23a |

---

## 15. Acceptance tests

Every test runs on reference hardware: Apple M1 8 GB on macOS 14.4 and the current macOS; a Windows 11 i5/16 GB laptop. Tests marked **live** run against a live workspace.

| AT | Name | Procedure (STE) | Pass criterion | Gate |
|---|---|---|---|---|
| AT-01 | Create two MOGs | Create two MOGs. | Both survive. | device |
| AT-02 | Recover expired sign-in | Expire a profile. | The config is unchanged. | live |
| AT-22 | Release administration gates | Run the spikes and legal review. | Evidence exists for each gate. | release |
| AT-31 | **MVP-D end to end** | Install and ask. | Every step passes. | **MVP exit**, live |

---

## 16. Week-1 spikes

| ID | Spike |
|---|---|
| SP-01 | Measure it. |
