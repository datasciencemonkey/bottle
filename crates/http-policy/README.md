# http-policy

The pure policy layer of Bottle's **HttpTransport** (component R7, Section 4.3 of
`research/final-requirements.md`). It decides where a bearer token may go, which
redirects may be followed with it, and whether an upstream response is a JSON API
answer or a non-JSON page. It does no network I/O: the real `reqwest` transport
calls it before every send and after every response.

Standalone crate (own empty `[workspace]`), one dependency: `url` (WHATWG parser).

## Requirements covered

| ID | What this crate does |
|---|---|
| SEC-04a | Token only to the exact profile host over https on 443 (host under `cloud.databricks.com`, `gcp.databricks.com`, `azuredatabricks.net`, `cloud.databricks.us`, `databricks.azure.us`) or to an allowlisted Databricks App URL (`https://*.databricksapps.com`, incl. the `aws.`, `<n>.azure.` and `gcp.` forms). `accounts.*` hosts are rejected (4.6 rules 1-2). |
| SEC-04b | A 3xx is followed with the token only to the same host, still https on 443, and only while it still passes the request check. Cross-host, downgrade, userinfo, other-port and loop redirects fail (4.6 rule 3). |
| SEC-05a | A response is JSON only when the media type is `application/json` or `application/*+json` and the body sniff starts with `{` or `[`. Anything else (HTML login, proxy, captive portal page) is `PolicyError::ProxyPage { status }`: code `PROXY_PAGE`, static card text, the body is never kept, shown or printed by `Display`/`Debug` (4.6 rule 6, Table W-ERR). |
| SEC-05b | A `PROXY_PAGE` is never retried with the token. `execute` retries only a JSON 502/503/504, at most `MAX_ATTEMPTS` sends in total, and records a content-free trace. |
| AT-17 | Malicious hosts and redirects are part of the tests. |

Normalisation edge cases tested: uppercase hosts and schemes, trailing dots
(refused as non-canonical), userinfo and `@`/`#`/`\` tricks, explicit `:443`
(accepted), other ports, http/ws/ftp downgrade, IDN (punycode, homographs never
match), IPv4 (dotted, hex, decimal) and IPv6 literals, suffix tricks such as
`evilcloud.databricks.com` and `<profile host>.evil.com`.

## API

```rust
use http_policy::{TokenPolicy, TokenTarget, classify_response, execute};

let policy = TokenPolicy::new("https://dbc-1234.cloud.databricks.com")?;   // profile host
policy.check_request(url)?;                       // -> TokenTarget::{ProfileHost, AppUrl}
policy.check_redirect(&current, location, hops)?; // -> RedirectDecision::Follow(next)
classify_response(status, content_type, body)?;   // -> ResponseClass::{Json, Empty}
let out = execute(&policy, url, |req| send(req)); // pure driver over an injected sender
```

- `PolicyError::code()` gives the `BackendErrorCode` of Section 4.7: `PROXY_PAGE`
  for a non-JSON page, `NETWORK_BLOCKED` for a refused redirect, `CLIENT_BUG` for a
  refused request URL. `card_text()` is static safe text; `may_retry()` is always
  false for these errors. No error carries a URL, header or body.
- `MAX_REDIRECTS` = 5, `MAX_ATTEMPTS` = 2.

## How to test

```sh
cd crates/http-policy
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Each test has a doc comment with the requirement IDs it verifies, checked by
`scripts/ci/check_req_tags.py`.

## Out of scope

- The `reqwest` + `rustls` client, disabling its automatic redirects and calling
  this crate: the R7 HttpTransport wiring in `mog-core` (#221 skeleton and its follow-ups).
- Request tags and User-Agent (SEC-06a/b), the per-profile rate bucket, AIMD,
  429/Retry-After and online/offline detection: other R7 issues.
- App URL workspace equals profile workspace (4.6 rule 1, v1): `custom_agent` work.
- Secret scrubbing of outbound strings (SEC-03, 4.6 rule 7): IpcGateway issue.
- Token storage and refresh (R6 TokenBroker).
