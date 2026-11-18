# ADR 0074: Model service access check and admin inspection

- Status: accepted
- Issue: #20 (ADM-04a, ADM-04b, ADM-09, AT-36). Soft dependency: #19 (AI Gateway model path).
- Date: 2026-11-17
- Owner: impl-20 (capture-lead-2)

## Context

The admin must give access to 7 model services (ADM-04a), turn on cross-geography routing only
for Databricks ASR (ADM-04b), and keep inference tables off on the ASR service (ADM-09). When
access is missing, Bottle must apply `SttPolicy=local_only` for ASR and the fallback classifier
for the agent. The requirements leave these points open:

- how Bottle finds out that a service refuses the home profile;
- what Bottle does for the cleanup, router, writer, judge and embedding services;
- what Bottle does when it cannot read the ASR inference-table config;
- what Bottle does with an MDM `SttPolicy` value that it does not know;
- where the ADM-09 inspection goes in the release checklist, which is generated.

`model_invoke` is not wired in the host yet (#19), and an IPC rejection carries only a fixed
code. No code tells "access denied" or "blocked by geography" apart from other failures.

## Decisions

1. **One check behind a probe interface** (`src/brain/components/model-access.ts`).
   `checkModelServices(input, probe)` probes the 7 services in parallel through a
   `ModelServiceProbe`. It returns a content-free report: the effective `SttPolicy` and its
   reason, the agent route, a fallback and a fixed status text for each service, and the
   `localOnly` and STT badge inputs. A probe that throws counts as `unreachable`. The report
   never holds a probe error message, a URL or a token.
2. **ASR is probed only when it is selected.** The check probes ASR only when the user selected
   Databricks ASR or MDM sets `cloud_only`, and MDM does not set `local_only`. Local STT is the
   default and needs no admin action (ADM-04b, CAP-57c). With `local_only`, no ASR probe leaves
   the device (CAP-60b).
3. **ASR fallback.** The effective policy is `local_only` when ASR has no access, is blocked by
   geography, cannot be reached, or has inference tables on. If the ASR probe passed but the
   inference-table config cannot be read (`unknown`), Databricks ASR stays on, and the status
   says that the tables are not verified. Users often cannot read the service config. The
   admin inspection (decision 7) covers this case.
4. **Fallbacks for the other services.** These follow ARC-F7: cleanup gives the pre-pass text
   only, the router step uses the heuristic fallback, embeddings wait in the queue, and the
   writer and judge are off. The main agent model gives the fallback classifier route (RTR-15b).
   A `not_checked` service gets no fallback, except ASR (decision 5).
5. **The default probe is not wired.** Until #19 lands, `createModelServiceProbe()` returns
   `not_checked` for each service. So Databricks ASR stays off (CAP-57e), and nothing else
   changes. `TODO(#19)`: the IPC probe needs a content-free IPC code for a refused service
   (access denied, blocked by geography) and a route that reads the ASR inference-table config.
   Those are contract changes that go through the contract lead.
6. **MDM keys.** `modelPolicyFrom` reads `SttPolicy` and the `*ModelService` keys of Appendix C.
   No `SttPolicy` value means `cloud_allowed`. An unknown value means `local_only`, the safe
   side.
7. **Admin inspection in the release checklist.** `scripts/gen_release_checklist.py` adds an
   "Admin inspection" table. It lists each MVP ADM row verified by inspection (Verify `I`),
   including ADM-09, with the page that has the steps. The checklist stays generated.

## Consequences

- The UC1 and UC2 paths can read one report at start and after a network change. They do not
  need to check access themselves. No code calls the check yet: `TODO(#52)` (SttProvider) and
  `TODO(#19)` (model path) mark where it gets wired.
- The live AT-36 run needs an admin and a real workspace (`needs:human`, `needs:admin` on #20).
- The admin page is `docs/admin/model-service-prerequisites.md`.
