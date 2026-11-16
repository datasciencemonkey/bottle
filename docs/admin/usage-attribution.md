# Usage attribution: Bottle User-Agent and request tags

Bottle marks its calls so that an admin can find them in usage tables (SEC-06a, SEC-06b, ADR 0023).

## What Bottle sends

| Header | Calls | Value |
|---|---|---|
| `User-Agent` | Every call | `bottle/<version> (<os>; mog)`, for example `bottle/0.1.0 (macos; mog)` |
| `Databricks-Ai-Gateway-Request-Tags` | Unity Gateway calls (model and ASR) and MCP calls | `{"app":"bottle","mog":"<MOG ID>","op":"<op>"}` |

`<os>` is `macos`, `windows`, `linux` or `other`. `op` is one of `asr`, `cleanup`, `router`, `writer`, `judge`, `embed`, `genie`, `merge`, `agent` or `page`. The MOG ID is a short slug. The tags contain no user name, transcript, token or URL.

## Find Bottle rows

Run this query in a SQL warehouse of the workspace. You need access to the `system.ai_gateway` schema.

```sql
SELECT event_time, service_type, endpoint_name, request_tags, user_agent
FROM system.ai_gateway.usage
WHERE event_time > current_timestamp() - INTERVAL 1 DAY
  AND (request_tags['app'] = 'bottle' OR user_agent LIKE 'bottle/%')
ORDER BY event_time DESC
LIMIT 50;
```

Each Bottle model row shows `request_tags` with the keys `app`, `mog` and `op`, and `user_agent` starts with `bottle/`. Spike SP-18 (#136) checks if MCP rows also keep the tags. If they do not, use the User-Agent to find MCP rows.
