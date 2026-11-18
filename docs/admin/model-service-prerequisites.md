# Model service prerequisites (admin)

Use this page to set up the model services that Bottle uses on the home workspace. It covers ADM-04a, ADM-04b and ADM-09, and items 5, 6 and 7 of the admin checklist (requirements section 10). The decision record is ADR 0074.

Bottle never picks a Databricks CLI profile for you. Each command below uses `--profile <your-profile>`. Replace it with the profile of the home workspace.

## 1. Give access to the model services (ADM-04a)

Bottle calls each model service through Unity Gateway (`/ai-gateway/mlflow/v1`) with a `system.ai.*` name (checklist item 5, NFR-21a). Give the group `bottle-users` permission to query each service in this table. Set the service tags `app=bottle,stage=<x>` on each service, where `<x>` is your stage (for example `dev` or `prod`), so that usage and budgets show Bottle (see `docs/admin/usage-attribution.md`).

| Service | What Bottle uses it for | MDM key for another service name | If access is missing |
|---|---|---|---|
| ASR | Databricks speech to text, only when a user selects it | `AsrModelService` | `SttPolicy=local_only` applies. Speech to text runs on this device. |
| cleanup | Cleanup of dictated text | `CleanupModelService` | Bottle keeps the text without cleanup (the pre-pass text). |
| router | The router step of each question | `RouterModelService` | Bottle uses a simple rule (the router fallback). |
| writer | Meeting notes | `WriterModelService` | Meeting notes are off. |
| judge | Answer checks | none | Answer checks are off. |
| embedding | Memory search | `EmbeddingModelService` | New memory waits in a queue. |
| main agent | The main agent model route (for example `/ai-gateway/anthropic`) | none | The fallback classifier routes each question (RTR-15b). |

To check the permission of one service:

```sh
databricks serving-endpoints get-permissions <service-name> --profile <your-profile>
```

The service name is the name in the AI Gateway page, for example `system.ai.gpt-oss-20b`. The group `bottle-users` must have `CAN QUERY`. For a Unity Catalog `system.ai.*` service, also grant `USE CATALOG system` and `USE SCHEMA system.ai` (checklist item 3).

Open question: the grant on a Unity Catalog `system.ai.*` model service itself (for example `EXECUTE`, as checklist item 3 uses for the MCP service) is not verified yet. Step 1 of the live admin check on #20 records it: https://github.com/datasciencemonkey/bottle/issues/20#issuecomment-6966537505. This page will name the grant after that step reports.

## 2. Cross-geography routing only for Databricks ASR (ADM-04b)

Local speech to text is the default (CAP-57c). It needs no admin action.

Do this step only if your users select Databricks ASR, or if MDM sets `SttPolicy=cloud_only`:

1. Open the account console.
2. Open the setting for cross-geography processing.
3. Turn on cross-geography routing for the workspaces where the Gemini ASR service needs it.

If you do not want audio to leave the workspace geography, do not turn it on. Set the MDM key `SttPolicy=local_only` instead (checklist item 6). If "Enforce data processing within workspace Geography" blocks Databricks ASR, Bottle also uses local speech to text (CAP-63).

## 3. Keep inference tables off on the ASR service (ADM-09)

Bottle keeps no copy of the audio that it sends for transcription (CAP-34). An inference table on the ASR service would keep a copy. So keep inference tables off on the ASR service (checklist item 7).

Inspect the service config:

```sh
databricks serving-endpoints get <asr-service-name> --profile <your-profile>
```

In the output, `ai_gateway.inference_table_config.enabled` must be `false` or absent. In the AI Gateway page of the service, "Inference tables" must show off. Link this output in the release PR (the "Admin inspection" table of `docs/release/RELEASE-CHECKLIST.md`).

## What Bottle does when a prerequisite is missing

The access check (`checkModelServices` in `src/brain/components/model-access.ts`, ADR 0074) is built and tested, but it is not run yet. Two issues must land first:

- #19 wires the model path through the AI Gateway. Until then, the probe cannot reach a service, so Databricks ASR stays off and local speech to text runs.
- #52 (SttProvider) and the UC2 path read the report of the check. Until then, no status shows in the app.

When the check runs, it will do these things. It will send no user content, and its status will hold no URL and no token.

- If Databricks ASR is selected and the ASR service has no access, Bottle will apply `SttPolicy=local_only`. The STT badge will show "STT: local" and "On this device".
- If cross-geography routing blocks the ASR service, Bottle will apply `SttPolicy=local_only`.
- If inference tables are on for the ASR service, Bottle will apply `SttPolicy=local_only`. If Bottle cannot read the config, the status will say that the inference tables are not verified.
- If the main agent model service has no access, the fallback classifier will route each question.
- If MDM sets `SttPolicy=local_only`, Bottle will send no ASR request and no ASR probe.

So the admin inspection of this page is the control until the check runs: inspect it for each release.
