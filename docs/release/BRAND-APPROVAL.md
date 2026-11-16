# Brand approval record

This file is the SEC-22b sign-off record for the MOG mascot and the Genie lamp artwork.
`scripts/check_brand_gate.py` reads it. The gate passes only when Status is `approved` and every
field below is filled. Approval is tracked in #129. Internal builds do not need it (COL-104). The
first public download needs it (SEC-22a, SEC-22b).

| Field | Value |
|---|---|
| Status | pending |
| Tracking issue | #129 |
| Approver | |
| Date approved | |
| Approval reference | |

## How to record the approval

1. Databricks brand (go/brand) answers the request in #129.
2. Attach the answer to #129.
3. Set Status to `approved`. Fill Approver, Date approved (YYYY-MM-DD) and Approval reference (a link or ticket ID).
4. In the release checklist, link this file in the Evidence cell of the AT-37 row, then tick the row.

## Art review checklist (SEC-22a, UX-10a..d)

- [ ] The mascot is an original character with no Codex likeness (art owner and brand reviewer).
- [ ] The lamp is an extension of the Genie logo, with brand approval (U14).
- [ ] The palette is the approved Genie palette, with no Lava FF3621 (UX-10b). Automated.
- [ ] No Tail, Wisp or Brick node (UX-10a, UX-10c). Automated.
- [ ] No `@rive-app/webgl` (NFR-16b). Automated.
- [ ] The view model is unchanged (UX-10d).
