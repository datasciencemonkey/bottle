# Motherbricks final director review

Date: 2026-11-14  
Verdict: **CHANGES_REQUIRED**

The document is self-contained, uses the correct Motherbricks name, keeps Lakebase optional, and honestly marks physical-device and live-workspace tests as pending. The following corrections are required before baseline approval.

1. **Resolve same-MOG capture.** C-16 and CAP-012 make stream reuse optional, while Figure 3 still requests director approval. Make the meeting owner's reuse of its owned stream the v1 baseline. Use separate PTT and meeting buffers. Mark the PTT interval. Do not open a second capture session. Align C-16, CAP-012, Figure 3 and AT-14. Remove the unresolved director note.

2. **Correct Genie API statements.** Figure 2 must not list `CANCELLED` as a documented Genie One MCP terminal poll state. Keep “cancellation requested” and reconcile the original response. Add workspace-ID resolution and `?o=<workspace-id>` when the route requires it. Replace the blanket “Beta” label for agent-mode operations with per-operation launch-status verification.

3. **Restore capture safety controls.** Require an independent persistent native indicator for hands-free capture and fail closed if it cannot remain visible. Add the 120-second PTT safety stop. Define `Active`, `Released` and `Unknown`; `Active` cancels the release grace period and `Unknown` never stops capture. Require process-scoped system audio and prohibit silent whole-system fallback. Add these cases to AT-03 and AT-13.

4. **Make MVP context sharing safe.** Move RTE-008 and RTE-009 to MVP because MVP can send context. Require source, timestamp, truncation state and a byte limit. Bind each approval to one immutable payload, target, conversation and submission attempt. Invalidate approval after any change. Move SEC-017 audit metadata to MVP and test these rules in AT-08.

5. **Reconcile token origins.** Limit WS-007's workspace-origin rule to workspace token leases. Preserve SEC-016's separate origin-bound lease for Databricks Apps. Require rejection of authenticated cross-origin redirects. Update the trust diagram so its workspace-only wording does not contradict the later Apps adapter.

6. **Repair release metadata and traceability.** Make G5 conditional on enabling Lakebase; it must not block a local-only MVP. Add MB-007 to AT-13 with an explicit pause-all assertion. Add MB-009 to AT-17 with an actual credential-free export test.

No existential user question is open. Physical macOS/Windows tests and live Databricks workspace tests remain release gates, not reasons to withhold the corrected requirements baseline.

---

## Re-review verdict — 2026-11-14

**APPROVED**

All six critical correction groups are resolved in the HTML and inline SVGs. The same-owner stream policy is decision-complete. Genie cancellation and workspace routing are accurate. Capture, context and token controls now match the specialist safety requirements. Lakebase remains optional, and G5 is conditional. AT-13 and AT-17 now cover the repaired requirements.

The document remains self-contained. Its four inline SVGs are present. No duplicate element IDs or missing internal link targets were found.

Residual gates are unchanged: physical macOS/Windows verification, live Databricks workspace verification, named-device benchmarks, applicable Lakebase tests, security/accessibility evidence and deployment legal review remain pending. These gates do not block approval of the requirements baseline.
