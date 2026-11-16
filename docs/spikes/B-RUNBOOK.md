# Run book B: live-workspace spikes SP-06, SP-07, SP-11, SP-12, SP-13, SP-14, SP-16, SP-20

Author: impl-b · Date: 2026-11-16 · Issues: #48, #54, #121, #191, #56, #167, #157, #187

Each section stands alone and goes on its issue as one comment. Each section repeats the shared
rules. A person runs it with a live
workspace and a U2M OAuth profile. The scripts were not run against a workspace. They pass offline
tests only (`uv run --no-project --with pytest pytest spikes`).

Rules for every section:

- You choose the profile. The scripts never choose one. Replace `<your-profile>` in each command.
- Run all commands from the repository root.
- The scripts get the token with `databricks auth token --profile <p>`. The token stays in memory.
  It goes only in an `Authorization` header to the profile host, or in the libpq password (SP-12).
  The scripts redact token-shaped strings from all output and reports.
- Each script writes a JSON report to the `--out` path that you give, and prints one
  `SP-nn PASS: ...` or `SP-nn FAIL: ...` line.
- `CHECK:` marks an assumption that I could not confirm from the public docs. Write the real value
  in the result comment.
- Post the printed PASS/FAIL line and the report file (or its `evaluation` block) on the issue.
  The report contains no token and no user content. Read it before you post it.

---

## SP-06 (#48): Unity Gateway audio MIME types and payload size

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A U2M profile for a workspace with Unity Gateway, `USE CATALOG system` and `USE SCHEMA system.ai`.
- Access to the model `system.ai.gemini-3-1-flash-lite` (CAP-62a, default ASR model). If cross-geo
  routing is off in your region, the model can return an error. Record that error.
- ffmpeg is optional: the script uses `soundfile` first and falls back to ffmpeg.

**Command**
```
uv run --no-project --with httpx --with soundfile --with numpy \
    spikes/sp-06/run.py --profile <your-profile> --out sp06.json
```
Optional: `--model system.ai.gemini-3-5-flash-lite` (the residency candidate, CAP-62c).

**What it sends** (to `POST /ai-gateway/mlflow/v1/chat/completions`, one `audio_url` data URI each)

| Case | Content |
|---|---|
| `30s-ogg`, `30s-opus`, `30s-flac`, `30s-wav` | 30 s synthetic speech-band clip at 16 kHz mono as `audio/ogg` (Vorbis), `audio/opus` (Ogg Opus), `audio/flac`, `audio/wav` |
| `size-3.5MB` | Request body of 3,500,000 bytes (WAV 44.1 kHz mono: 29.8 s) |
| `size-4.1MB` | Body of 4,100,000 bytes (WAV: 34.9 s) |
| `size-4.1MiB` | Body of 4,299,162 bytes (WAV: 36.6 s). This case shows if the limit uses MB or MiB. |

The size cases use the first size-controllable format whose 30 s case was accepted: WAV, else FLAC
of white noise (FLAC cannot compress noise, so the size follows the duration). If neither WAV nor
FLAC is accepted, the size cases do not run and the result is `NOT PROVEN`.

The clips come from a fixed synth. They hold no speech. The report keeps only the length of the
model output, not its text.

**Expected output**
```
  30s-ogg      status=200 outcome=accepted body=60xxx
  ...
  size-4.1MB   status=4xx outcome=rejected_clean body=4099xxx
SP-06 PASS: formats_known=True accepted_formats=[...] size_format=wav 3.5MB_accepted=True 4.1MB_rejected_cleanly=True 4.1MiB=...
```

**Pass/fail rule (issue #48)**
Pass when all of these are true:
- Each of the four formats has a recorded status (no transport error).
- The 30 s case of the size format (WAV or FLAC) is accepted.
- The 3.5 MB body in that format is accepted.
- The 4.1 MB body in that format gets a 4xx answer with a JSON error body ("rejected cleanly").

A 5xx, an HTML page or a dropped connection is not a clean reject. The report keeps the error shape
(status, JSON keys, `error_code` and a short redacted message) for each case.

**Fallback (fail)**: use the accepted format; lower segments to 20 s.

**CHECK:**
- The docs list no audio MIME types and no size limit. The docs show `data:audio/mp3;base64,...`.
- The 3.5 MB and 4 MB figures come from the requirements (CAP-53c, COL-23). MB means 1,000,000 bytes here.
- The over-cap cases are longer than 30 s, because the size grows with duration. The gateway
  does not have a 30 s rule. The 30 s cap (CAP-53a) is a Bottle rule.

**Time**: about 10 minutes.

**Docs used**: https://docs.databricks.com/aws/en/machine-learning/model-serving/query-audio-video-models,
https://docs.databricks.com/aws/en/machine-learning/model-serving/query-chat-models,
https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/supported-models

---

## SP-07 (#54): FMAPI cleanup and router latency from the desktop

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A U2M profile, run from a desktop on a normal office or home network (not a VPN to another region,
  if you can avoid it). Record your city and network type in the comment.
- Access to `system.ai.meta-llama-3-1-8b-instruct`, `system.ai.gpt-oss-20b`, `system.ai.gemini-3-1-flash-lite`.
  Llama 3.1 8B is not served in all regions (COL-51). If a model gives errors, record that.

**Command**
```
uv run --no-project --with 'httpx[http2]' spikes/sp-07/run.py --profile <your-profile> --out sp07.json
```
Options: `--kinds cleanup` (cleanup only), `--models <a,b>`, `--n 200`, `--warmup 5`,
`--pause-ms 50` (if you get 429 answers).

**What it does**
- One warm httpx client with HTTP/2. 5 warm-up calls per model and kind are dropped.
- 200 non-streaming calls per model, for two kinds:
  - `cleanup`: fixed synthetic dictation, `max_tokens` 200.
  - `router`: the strict Appendix D json_schema (`spikes/sp-13/router_schema.json`).
- Latency is the time from the send of the request to the read of the full body.
- The report has the HTTP version of each call, so you can confirm HTTP/2.

**Expected output**
```
SP-07: cleanup system.ai.meta-llama-3-1-8b-instruct x200 ...
  p50=... p95=... errors=0 http=['HTTP/2'] ok
...
SP-07 PASS: p95 <= 800 ms for all; failing=[] fastest={...}
```

**Pass/fail rule (issue #54)**
- Pass when p95 ≤ 800 ms for each model and kind, and every measured call used HTTP/2
  ("on warm HTTP/2"). A run on HTTP/1.1 fails, also when it is fast.
- A model with fewer than 95 % successful calls fails, because there are not enough samples.
- The report gives p50, p95, minimum and maximum for each model and kind.

**Fallback (fail)**: pre-pass only on PTT; router on the faster model. The output names the fastest
router model.

**CHECK:**
- The model names come from the requirements and the supported-models page. Region availability
  can differ.
- That the gateway offers HTTP/2. If the script prints `http=['HTTP/1.1']`, the criterion fails.
  Record it, and check for a proxy on your network that ends HTTP/2.

**Time**: about 20 minutes (1,200 calls).

**Docs used**: https://docs.databricks.com/aws/en/machine-learning/model-serving/query-chat-models,
https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/supported-models,
https://docs.databricks.com/aws/en/machine-learning/model-serving/structured-outputs

---

## SP-13 (#56): strict json_schema router on small models

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A U2M profile with access to `system.ai.meta-llama-3-1-8b-instruct` and `system.ai.gpt-oss-20b`.
  For the fallback check, also `system.ai.gemini-3-1-flash-lite`.

**Inputs in the repository**
- `spikes/sp-13/router_schema.json`: Appendix D, copied verbatim. A test checks that it still
  equals Appendix D. PR #293 is not merged, so this file does not reuse its schema.
- `spikes/sp-13/golden.json`: 500 synthetic items from `golden.py` (92 fixed templates, no LLM).
  Each item has an expected intent, command and injection flag, a capture mode and a speaker track.

| Group | Items | Mode and track | Expected |
|---|---|---|---|
| question | 130 | PTT / hands-free, you | `question` |
| memory_question | 60 | PTT / hands-free, you | `memory_question` |
| command | 90 | PTT / hands-free, you | `command`, 9 for each command value, `other` included |
| dictation | 70 | PTT / hands-free, you | `dictation` |
| dictation with a question | 30 | PTT / hands-free, you | `dictation` (message text with a question to a person) |
| context | 50 | meeting, you | `context` |
| noise | 30 | meeting, you | `noise` |
| Others track | 20 | meeting, others | `noise` with `injection_suspected=true` (RTR-08) |
| injection | 20 | PTT / hands-free, you | `injection_suspected=true`; intent not scored (RTR-14) |

**Label policy** (`golden.py`, L1-L6; the system prompt says the same):
- L1: in PTT and hands-free mode the intent is `question`, `memory_question`, `command` or
  `dictation`. Every other utterance is `dictation` (RTR-01b). So these modes never use `context`
  or `noise`.
- L2: text for an email or a message to a person is `dictation`, also when it holds a question to
  that person.
- L3: in meeting mode `dictation` does not apply. Background is `context`, filler is `noise`.
- L4: a command or a request from the Others track is `noise` with the injection flag (RTR-08).
- L5: injection items (Table RTR-INJ) are scored on the flag only.
- L6: `other` is a request to the assistant with no named command.

**Command**
```
uv run --no-project --with httpx spikes/sp-13/run.py --profile <your-profile> --out sp13.json
```
Smoke test first (optional): `--limit 20`. Fallback model: `--models system.ai.gemini-3-1-flash-lite`.
Rate limits: `--workers 2`.

**Expected output**
```
SP-13: system.ai.meta-llama-3-1-8b-instruct on 500 utterances ...
  valid=99.80% accuracy=93.40% http_errors=0 ok
...
SP-13 PASS: system.ai.meta-llama-3-1-8b-instruct valid=...; system.ai.gpt-oss-20b valid=...
```

**Pass/fail rule (issue #56)** for each model:
- The valid-JSON rate must be ≥ 99.5 %. "Valid" means that the answer parses and matches the
  schema: all required keys, no extra keys, the types and the enums.
- The intent accuracy must be ≥ 90 % over the 480 intent-scored items (all but the 20 injection
  items). An invalid answer counts as wrong.
- A call that still fails after 3 retries (429 or 5xx) counts as invalid.

The report also gives these results. They are not part of the gate:
- the result of each group, with the wrong intents that the model chose,
- the injection flag rate on the 40 Others-track and injection items,
- the command accuracy,
- the latency.

**Fallback (fail)**: `gemini-3-1-flash-lite` for the router where allowed. Run the command again
with `--models system.ai.gemini-3-1-flash-lite` and post the result.

**CHECK:**
- The label policy L1-L6 is my reading of RTR-01b, RTR-08, RTR-14 and Appendix D. The router owner
  must confirm it, in particular L2 (dictation with a question) and L3 (meeting mode).
- The system prompt and the router input are my text. If a label looks wrong, record the item IDs.
- Limit: the 92 templates repeat their slot values. One misread template can move the accuracy by
  up to 2.7 %. The per-group table shows this.

**Time**: about 20 minutes (1,000 calls with 4 workers).

**Docs used**: https://docs.databricks.com/aws/en/machine-learning/model-serving/structured-outputs

---

## SP-11 (#121): Genie One API list and delete of MCP threads

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A workspace admin turns on the **Genie One API** preview (ADM-14).
- A U2M profile with `all-apis` (default). Genie One MCP needs scope `ai-gateway`. The Genie One
  API needs scope `genie`.
- `EXECUTE` on `system.ai.genie_one_mcp` (ADM-01b), and at least consumer access.

**Command**
```
uv run --no-project --with httpx spikes/sp-11/run.py --profile <your-profile> --out sp11.json
```
If the script cannot read the workspace ID, add `--workspace-id <id>`. The ID is the number after
`?o=` in the workspace URL.

**What it does**
1. It creates a thread through Genie One MCP. It sends `initialize`, `tools/list`, then `genie_ask`
   with the question "Bottle SP-11 probe: which catalogs can I see?". Then it polls
   `genie_poll_response` for up to 90 s.
2. `databricks api get "/api/2.0/genie/one/conversations?page_size=100"`: it looks for the thread
   on up to 5 pages.
3. `databricks api delete /api/2.0/genie/one/conversations/<id>`.
4. It lists again and GETs the thread. The thread must be gone.

The script deletes only the thread that it made. It never prints conversation titles.

Manual variant: make a thread yourself in an MCP client (for example the AI Playground with the
Genie One MCP server). Copy its conversation ID and run:
```
uv run --no-project --with httpx spikes/sp-11/run.py --profile <your-profile> \
    --conversation-id <id> --i-created-it --out sp11.json
```

**Expected output**
```
SP-11 PASS: listed=True deleted=True absent_after=True -> Genie One API delete utility (G-45a)
```

**Pass/fail rule (issue #121)**: pass when the MCP thread is listed, the delete succeeds, and after
the delete the thread is not listed and GET fails.

**Fallback (fail)**: "Delete in Genie One" deep link with a disclosure (G-45b).

**CHECK:**
- The arguments of `genie_poll_response` (`conversation_id`, `response_id`) are not on the docs page.
- The MCP protocol version is `2025-06-18`. The server answer is recorded.
- The workspace ID comes from the `x-databricks-org-id` response header.

**Time**: about 10 minutes, plus the admin step.

**Docs used**: https://docs.databricks.com/aws/en/genie-one/api,
https://docs.databricks.com/api/genie-one/v1/delete-conversation,
https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp

---

## SP-12 (#191): Lakebase cold start and vectors

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A U2M profile in a workspace with Lakebase (autoscaling projects). You must be allowed to create
  a project.
- **Admin or project-owner step:** turn on **Lakebase Search** for the new project (project
  settings in the Lakebase UI). `lakebase_ann` needs it. If Search stays off, the script reports
  the `lakebase_ann` rows as `NOT RUN` (not `FAIL`).
- `CREATE EXTENSION lakebase_vector` is irreversible for the database (Appendix A, migration
  0002). The script asks before it, before the resize and before `DROP SCHEMA`. Use the new
  throwaway project. The script refuses a project that it did not create, unless you add
  `--project-id <id> --allow-existing-project`.

**Commands** (two steps)
```
# Step 1: create the throwaway project and stop.
uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> \
    --create-only --out sp12-create.json

# Admin step: turn on Lakebase Search for the project named in the output (bottle-sp12-xxxxxx).

# Step 2: measure. The script asks three questions; type yes to each, or add --yes.
uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> \
    --project-from sp12-create.json --delete-project --out sp12.json
```
Options: `--rows 5000`, `--queries 200`, `--cold-runs 3`, `--suspend-wait-s 420`, `--skip-resize`,
`--keep-schema`, `--yes`.

**What it does**
1. Step 1 runs `databricks postgres create-project bottle-sp12-<suffix> --json '{"spec":{...}}'` and
   records the JSON shape (keys and types only).
2. Step 2 finds the read-write endpoint and records its old CU and suspend values. After you
   confirm, it sets 0.5 CU minimum and maximum and a 60 s suspend timeout with `update-endpoint`.
3. It gets a credential with `databricks postgres generate-database-credential <endpoint>`. The
   credential is the password. It stays in memory.
4. TLS probe: TCP, Postgres SSLRequest, then a TLS handshake with the system CA store and the host
   name check. This is the `sslmode=verify-full` chain.
5. It connects with psycopg (`sslmode=verify-full`, `sslrootcert=system`, `connect_timeout=5`,
   database `databricks_postgres`). It records the connect time (TCP + TLS + OAuth login) and the
   `SELECT 1` time. If verify-full fails, it records the error and tries `sslmode=require`.
6. It stops if schema `bottle_sp12` already exists. It makes tables with `vector(1024)` and
   `halfvec(1024)` (Appendix A columns), fills them on the server and adds the GIN index on
   `text_tsv` (Appendix A `memory_item_tsv_gin`). It builds HNSW (MVP form). After you confirm the
   extension, it builds `lakebase_ann` (migration 0002). For each index it runs the Appendix A
   hybrid query 200 times warm (vector CTE + lexical CTE, `LIMIT 40`) and records p50 and p95.
7. After you confirm, it drops schema `bottle_sp12`.
8. Cold wake, 3 times: it waits until `get-endpoint` shows a suspended state. Then it times connect
   plus `SELECT 1`. The credential is fetched before the timer starts.
9. `--delete-project` deletes the project only if this script created it. It records the real
   result of the delete.

**Expected output**
```
SP-12: project=bottle-sp12-ab12cd endpoint state=ACTIVE
  hnsw:vector: built=True hybrid=...
  lakebase_ann:vector: built=True hybrid=...
  cold run 1: {... 'wake_ms': ...}
SP-12 PASS: chain=True cold_max=... ann_builds=True ann_not_run=False hybrid_p95=... (hnsw ...)
  project delete: {'deleted': True, 'returncode': 0}
```

**Pass/fail rule (issue #191)**: pass when all of these are true:
- The `lakebase_ann` 1024-d hybrid query has a warm p95 ≤ 300 ms.
- The worst cold wake is ≤ 1 s.
- `lakebase_ann` builds on `vector(1024)` and on `halfvec(1024)`.
- The verify-full chain verifies, in the TLS probe and in the psycopg login.

`NOT RUN`: `lakebase_vector` was not available (Lakebase Search off) or not confirmed, and every
other criterion passed. Turn on Search and run step 2 again.

**Fallback (fail)**: pgvector HNSW stays; `sslmode=require`; prewarm on key-down. The output names
the fallback for each failed criterion.

**Cleanup**
- The script drops the schema, unless you give `--keep-schema`.
- If the run stops early, it writes a partial report with `project_id` and `created_by_script`.
- If `created_by_script` is true and the project still exists, delete it with the script. It
  deletes only a `bottle-sp12-*` project that it created:
  `uv run --no-project spikes/sp-12/run.py --profile <your-profile> --delete-project-from sp12-create.json --out sp12-cleanup.json`.
  Or by hand: `databricks postgres delete-project projects/<project_id> --profile <your-profile>`
  (add `--purge` to skip the 7-day soft delete).
- Do not delete a project that the script did not create.

**CHECK:**
- The `update-endpoint` update mask syntax (`spec.autoscaling_limit_min_cu,...`).
- The host field `status.hosts.host`. The reference shows `hosts` as `{}`.
- The suspended state names in `status.current_state` (the script accepts SUSPENDED, IDLE, INACTIVE).
- `halfvec_cosine_ops` for `lakebase_ann`. The docs page shows only `vector_*_ops`.
- The cast from `double precision[]` to `halfvec`.
- That `generate-database-credential` does not wake the compute.

**Time**: about 30 minutes: 2 minutes for step 1, a few minutes for the admin step, and about
20 minutes for step 2. Most of step 2 is the 3 suspend waits. With `--cold-runs 2` it is shorter.

**Docs used**: https://docs.databricks.com/aws/en/oltp/projects/cli,
https://docs.databricks.com/aws/en/oltp/projects/manage-projects,
https://docs.databricks.com/aws/en/oltp/projects/authentication,
https://docs.databricks.com/aws/en/oltp/projects/scale-to-zero,
https://docs.databricks.com/aws/en/oltp/projects/lakebase-vector,
https://docs.databricks.com/api/postgres/v1/generate-database-credential.md,
https://docs.databricks.com/api/postgres/v1/get-endpoint.md

---

## SP-14 (#167): fine-grained CLI OAuth scopes

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- Check the CLI with `databricks --version`. In v1.25, `--scopes` takes a **comma-separated**
  list (`databricks auth login --help`).
- A browser for the U2M login. You can use the workspace host of your normal profile.

**Scope lists to test.** The issue and the requirements differ (spike-lead posted the difference on #167):
- W-19 (requirements): `ai-gateway,genie,knowledge,offline_access`.
- Issue #167: `ai-gateway,genie,postgres,offline_access`.

**Commands**
```
databricks auth login --host https://<your-workspace-host> --profile bottle-sp14-w19 \
    --scopes ai-gateway,genie,knowledge,offline_access
uv run --no-project --with httpx spikes/sp-14/run.py --profile bottle-sp14-w19 \
    --workspace-id <id> --domain-id <home-domain-id> --out sp14-w19.json

databricks auth login --host https://<your-workspace-host> --profile bottle-sp14-issue \
    --scopes ai-gateway,genie,postgres,offline_access
uv run --no-project --with httpx spikes/sp-14/run.py --profile bottle-sp14-issue \
    --workspace-id <id> --domain-id <home-domain-id> --out sp14-issue.json
```
- Give `--workspace-id`. The automatic lookup uses a SCIM call, and SCIM can fail with narrow scopes.
- `--domain-id` is optional. With a domain, the pages call creates a DRAFT probe Page and deletes it
  at once. Without a domain, the pages call is `GET /api/2.0/knowledge/pages`.

**Calls**

| Call | Path | MVP |
|---|---|---|
| `mcp_initialize`, `mcp_tools_list` | `POST /ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<id>` | yes |
| `model_call` | `POST /ai-gateway/mlflow/v1/chat/completions` (Llama 3.1 8B, `max_tokens` 5) | yes |
| `knowledge_pages` | `POST /api/2.0/knowledge/pages?request_id=...` (DRAFT), then `DELETE`; or `GET` list | yes |
| `genie_one_api_list` | `GET /api/2.0/genie/one/conversations` | info (v1) |
| `postgres_projects_list` | `GET /api/2.0/postgres/projects` | info (v1) |

The script reads the `scope` claim of the token in memory. It prints only the scope list.

**Expected output**
```
SP-14: token format=jwt scopes=['ai-gateway', 'genie', 'knowledge', 'offline_access']
  mcp_initialize           ok=True status=200
  ...
SP-14 PASS: scopes=[...] (token scopes match W-19) failing=[] info={"genie_one_api_list": true, "postgres_projects_list": false}
```

**Pass/fail rule (issue #167)**: for each profile, all of these are true:
- Every MVP call succeeds.
- The `scope` claim of the token equals the list under test (W-19, the issue list, or
  `--expect-scopes a,b,c`), with no `all-apis`.

If the calls succeed but the token is opaque, has no scope claim, has `all-apis` or has another
list, the result is `NOT PROVEN`: the calls may have passed with a wide token. Log in again with
`--scopes` and run again. The W-19 list must pass for W-19 to apply.

**Fallback (fail)**: stay on `all-apis`.

**Cleanup**: `databricks auth logout --profile bottle-sp14-w19` and the same for `bottle-sp14-issue`.

**CHECK:**
- The token is a JWT with a `scope` claim. If it is opaque, the script prints `format=opaque` and
  the result is `NOT PROVEN`.
- A 403 on the pages GET list can mean a missing MANAGE DISCOVER permission, not a missing scope.
  Use `--domain-id` to avoid this.

**Time**: about 20 minutes (two browser logins).

**Docs used**: `databricks auth login --help` (CLI v1.25.0, run locally),
https://docs.databricks.com/aws/en/dev-tools/cli/reference/auth-commands,
https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp,
https://docs.databricks.com/api/knowledge/v1/page

---

## SP-16 (#157): token type reaching Databricks Apps

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A U2M profile with permission to create a Databricks App, and `CAN MANAGE` on it.
- **Admin step:** user authorization for Databricks Apps must be available in the workspace.
  CHECK: it can be a workspace preview that an admin turns on (Previews page). The workspace
  setting `allowedAppsUserApiScopes` must allow the scope that you add (default: all).
- The probe app is in `spikes/sp-16/app/` (`app.py`, `app.yaml`). It is stdlib only.
- It logs and returns only header NAMES, and for a JWT only the `typ`/`alg` header fields and the
  `scope`, `scp`, `token_type`, `typ`, `grant_type` claims. It decodes the token in memory. It never
  logs or returns a token value, an email or an IP address.

**Deploy**
```
databricks apps create bottle-sp16 --profile <your-profile>
databricks sync spikes/sp-16/app /Workspace/Users/<you>/bottle-sp16 --profile <your-profile>
databricks apps deploy bottle-sp16 --source-code-path /Workspace/Users/<you>/bottle-sp16 --profile <your-profile>
databricks apps get bottle-sp16 --profile <your-profile>      # check that "url" is set
```

**Run 1: user authorization ON.** Add a scope in the app settings (UI: app → Settings → User
authorization → Add scope, for example `sql`), or run:
```
databricks apps update bottle-sp16 --json '{"user_api_scopes": ["sql"]}' --profile <your-profile>
databricks apps deploy bottle-sp16 --source-code-path /Workspace/Users/<you>/bottle-sp16 --profile <your-profile>
uv run --no-project --with httpx spikes/sp-16/run.py --profile <your-profile> \
    --app-name bottle-sp16 --mode on --out sp16-on.json
```

**Run 2: user authorization OFF.**
```
databricks apps update bottle-sp16 --json '{"user_api_scopes": []}' --profile <your-profile>
databricks apps deploy bottle-sp16 --source-code-path /Workspace/Users/<you>/bottle-sp16 --profile <your-profile>
uv run --no-project --with httpx spikes/sp-16/run.py --profile <your-profile> \
    --app-name bottle-sp16 --mode off --out sp16-off.json
uv run --no-project spikes/sp-16/run.py --merge sp16-on.json sp16-off.json --out sp16.json
```
The client reads the app URL with `databricks apps get bottle-sp16 --profile <your-profile>`. That
call answers only for an app in the profile's workspace, so the app workspace equals the profile
workspace (requirements 4.6 rule 1). The URL must also match `https://*.databricksapps.com`. If you
also give `--app-url`, it must equal that URL. Then the client calls `POST <url>/api/invocations`
with the CLI token in a Bearer header.

Also check the app log: `databricks apps logs bottle-sp16 --profile <your-profile>`. It shows
lines `sp16 {...}` with names and claims only.

**Expected output**
```
  app saw headers: ['x-forwarded-access-token', 'x-forwarded-email', ...]
SP-16[on] PASS: jwt typ=... scope=[...]
SP-16[off] PASS: no user token forwarded
SP-16 PASS: {"on": {...}, "off": {...}}
```

**Pass/fail rule (issue #157)**: pass when the token type and scope are known for both modes:
- ON: a forwarded user token arrives, and its format and scope are known.
- OFF: the result is recorded.

An opaque ON token fails, because its scope is not known.

**Fallback (fail)**: origin-bound lease per app; v1 gating.

**Cleanup**: `databricks apps delete bottle-sp16 --profile <your-profile>`.

**CHECK:**
- The `user_api_scopes` update payload. `databricks apps update` is a full replacement (CLI help):
  fields that you omit are cleared. On this probe app only the scopes matter. The UI path is safer.
  Read the app first with `databricks apps get` if you want to keep other fields.
- That an update needs a new deploy before the change applies.
- The command `databricks sync` for upload. `databricks workspace import-dir` also works.
- That the app does not get its own `/api/invocations` auth gate.

**Time**: about 25 minutes (two deploys).

**Docs used**: https://docs.databricks.com/aws/en/dev-tools/databricks-apps/auth,
https://docs.databricks.com/aws/en/dev-tools/databricks-apps/http-headers,
https://docs.databricks.com/aws/en/dev-tools/databricks-apps/connect-local

---

## SP-20 (#187): UC Pages API, scopes, DRAFT grounding and MCP visibility (with SP-20b)

**Shared rules (the same in every section)**
- Run every command from the root of a checkout of the repository. You need `uv` and the Databricks CLI ≥ 1.25 on the PATH.
- You choose the profile. The script never chooses one. Replace `<your-profile>`.
- The token comes only from `databricks auth token --profile <p>`. It stays in memory, in an `Authorization` header to the profile host only (SP-12: the libpq password).
- The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.
- `CHECK:` marks an assumption that the public docs did not confirm. Write the real value in your result comment.

**Prerequisites**
- A workspace admin turns on the **Unity Catalog Pages** Beta (ADM-21).
- One domain exists. Copy its ID from Catalog Explorer. (Without `--domain-id`, the script calls
  `GET /api/2.0/domains`, prints the status and count, and stops. It never picks a domain.)
- Two users: you (the owner) and a second user in the same workspace. The second user is needed
  for the grounding check.
- Three U2M profiles. Make them with these commands:
```
databricks auth login --host https://<host> --profile bottle-sp20-all  --scopes all-apis,offline_access
databricks auth login --host https://<host> --profile bottle-sp20-know --scopes knowledge,offline_access
databricks auth login --host https://<host> --profile bottle-sp20-dom  --scopes knowledge,domains,offline_access
```

The work has two parts. Each part takes less than 30 minutes. One person can do part 1 alone.
Part 2 needs the second user.

**Part 1: automated steps (about 20 minutes: three logins and four runs)**
```
uv run --no-project --with httpx spikes/sp-20/run.py --profile bottle-sp20-all  --domain-id <id> --out sp20-all.json
uv run --no-project --with httpx spikes/sp-20/run.py --profile bottle-sp20-know --domain-id <id> --no-keep --out sp20-know.json
uv run --no-project --with httpx spikes/sp-20/run.py --profile bottle-sp20-dom  --domain-id <id> --no-keep --out sp20-dom.json
```
Each run does these steps:
1. `GET /api/2.0/domains` (COL-100). It records the status and the count.
2. `POST /api/2.0/knowledge/pages` with `state: "DRAFT"` and `request_id` in the JSON body (the B3
   form; the requirements win), two times with the same ID. Both answers must name the same Page.
   This form decides the pass.
3. The same pair with `request_id` as a query parameter (the API docs form). The result is recorded
   only. The docs and the requirements differ (spike-lead posted the difference on #187). If only
   the query form works, the script prints a NOTE for #187.
4. It keeps one Page for the manual check: the first form (body, then query) that made exactly one
   Page. It deletes every other Page that the run made. `--no-keep` keeps none.

The script deletes a Page only if it created it in this run, and only if the Page `display_name`
has the marker "Bottle SP-20 probe".

**Part 2: manual steps (about 20 minutes, with the second user)**

**Manual step A: DRAFT grounding (owner only).** The kept Page defines the made-up term
**Bottle quuxrate** ("the count of blue widgets divided by the count of red widgets in one week").
1. As the owner, open Genie One. Ask: "What does Bottle quuxrate mean?" Expected: the answer uses
   the definition.
2. As the second user, ask the same question. Expected: the answer does not have the definition.
3. Record both answers as "used the definition: yes or no". Do not paste other content.

**Manual step B (SP-20b): MCP conversation visibility (COL-103).**
```
uv run --no-project --with httpx spikes/sp-20/run.py --profile bottle-sp20-all --domain-id <id> \
    --mcp-check --no-keep --workspace-id <id> --out sp20-mcp.json
```
This makes one Genie One MCP conversation with the question "Bottle SP-20b probe: what does Bottle
quuxrate mean?". Open the Genie One desktop app and the web history. Record if the conversation
shows there (yes or no). If the Genie One API preview is on, the report also has
`listed_in_genie_one_api`.

**SP-20b cleanup (optional).** The probe conversation holds only the synthetic question. To delete
it, use the Genie One UI (conversation menu → Delete), or, with the Genie One API preview on:
`databricks api delete /api/2.0/genie/one/conversations/<conversation_id from sp20-mcp.json> --profile bottle-sp20-all`.
Delete only that ID.

**Cleanup of the kept Page**
```
uv run --no-project --with httpx spikes/sp-20/run.py --profile bottle-sp20-all \
    --cleanup-from sp20-all.json --out sp20-cleanup.json
```

**Expected output**
```
SP-20 PASS: scopes=['all-apis', ...] body_form: status=200 draft=True idempotent=True; query_form (recorded): status=... idempotent=...; domains_readable=...
  kept for grounding: ['pages/...']  term: Bottle quuxrate
```

**Pass/fail rule (issue #187)**
- The automated run, for each profile, passes on 200 with state DRAFT and an idempotent retry with
  `request_id` in the body (B3). The query form is recorded for #187.
- The spike passes when the `all-apis` run and the `knowledge`-only run pass, and Genie One uses
  the DRAFT for the owner only (manual step A).
- Domains: if `domains_readable` is false, the picker uses a pasted domain ID or URL (PG-20).
- MCP visibility: if the conversation does not show in Genie One, the blurb deep link stays the
  route (U3, COL-103).

**Fallback (fail)**: keep Page drafts local with Copy as markdown; ask the account team.

**CHECK:**
- `request_id` location: the API docs put it in the query string, and B3 lists it with the body
  fields. The body form decides the pass; both forms are recorded.
- The scope name `domains` for `GET /api/2.0/domains`.
- That `knowledge` alone is enough for create and delete.

**Time**: about 40 minutes in total, in two parts of about 20 minutes each (part 1 alone, part 2
with the second user). Plus the admin step for the Beta if it is not on.

**Docs used**: https://docs.databricks.com/api/knowledge/v1/page,
https://docs.databricks.com/aws/en/uc-semantics/pages,
https://docs.databricks.com/api/domains/v1/domain,
https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp
