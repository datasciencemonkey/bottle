# ADR 0026: One source for the Table SEC-DISC disclosures

- Status: accepted
- Issue: #93 (SEC-17, AT-24)
- Date: 2026-11-16

## Context

SEC-17 says the UI must disclose each item of Table SEC-DISC in onboarding or the data panel.
The issue did not say where the copy lives, how the hub and onboarding share it, or how the
inspection test finds the items. Table SEC-DISC has no row IDs. The onboarding wizard (#43) does
not exist yet. The per-MOG "Where your data goes" panel built from the live config is #94
(UX-24a), and #94 depends on #93.

## Decision

1. The copy lives in one file, `src/views/hub/disclosures.ts`. It is not a contract file.
   Each entry has the verbatim item text of Table SEC-DISC as its key, a DOM id, a title,
   a v1 flag and plain-text sentences.
2. `src/views/hub/DataDisclosures.tsx` renders the list. `DataPanel` is the hub section
   "Where your data goes", mounted in `Hub.tsx`. `OnboardingDisclosures` shows the same list
   for setup; its mount point is `TODO(#43)`.
3. The inspection test reads the line `**Table SEC-DISC ...**` from
   `research/final-requirements.md` and splits it on ` · `. The set of items must equal the set
   of entry keys. A new or changed row fails the test until it has copy.
4. The copy is plain text, rendered as JSX text. There is no markdown and no
   `dangerouslySetInnerHTML`. Each sentence has 25 words or less.
5. The static disclosures are separate from the #94 live panel. #94 can embed `DisclosureList`.
6. Where Appendix E and the decisions differ, the copy follows the decisions. Appendix E says
   Bottle deletes Genie conversations after 90 days; U9 says Bottle deletes no user content
   automatically. The Genie history copy follows U9.

## Consequences

- A change to Table SEC-DISC needs a change to `disclosures.ts` and to
  `docs/user/where-your-data-goes.md`. The test checks both.
- The copy is static. Values from the live config or MDM (for example the STT badge or the
  Lakebase PITR value) belong to #94.
