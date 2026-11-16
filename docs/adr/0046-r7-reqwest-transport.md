# ADR 0046: The real R7 HttpTransport on reqwest and rustls

- Status: Accepted
- Date: 2026-11-16
- Issue: #364 (for #197, #207, #138)
- Owner: impl-364 for contract-lead

## Context

ADR 0041 (#342) gave `HttpTransport::request(profile, route_id, params, body, tags)`, the fixed map `http_transport::ROUTES` and `outbound_headers`. Issue #364 asks for the real client. These inputs are not on main when this work starts:

- PR #256 (#207, `crates/http-policy`) is open.
- #138 (the rate bucket) is open.
- The R5 profile read (#16, #17) is a stub.

## Decisions

1. **Base.** The work started on `issue/342-transport-tags` (PR #370), because it needs the #342 trait. PR #370 merged first, so the branch is now on `origin/main`. This PR does not change the trait.
2. **Crates.** We use `reqwest =0.13.5` with `default-features = false` and the features `blocking` and `rustls-no-provider` only. We use `rustls =0.23.46` with the `ring` provider (`std`, `tls12`). R7 installs the ring provider once per process. We use ring because `aws-lc-rs` needs NASM on Windows, and CI is Windows. The stack has no native-tls, no OpenSSL, no HTTP/2, no cookies and no decompression. Certificates come from the OS trust store (`rustls-platform-verifier`). This supports TLS-inspecting company proxies.
3. **Proxies.** R7 reads `HTTPS_PROXY` and `NO_PROXY` from the environment (reqwest default). It does not read the OS proxy settings, because the `system-proxy` feature is off. If users need OS proxy settings, a follow-up adds the feature.
4. **Route table.** `http_transport::routes::ROUTE_TABLE` gives each wired route ID a fixed method and a path template. The paths come from 4.6, NFR-21a, PG-20 and the linked Databricks REST pages. `params` fill only the named `{slot}`s and the listed query keys. A path value must match `[A-Za-z0-9_-]{1,128}`. A query value is a short token or a non-negative integer, and R7 percent-encodes it. Any other value gives `invalid_params`. Missing and unknown params also give `invalid_params`.
5. **Routes that are not wired.** These routes give `route_not_wired`:
   - `apps.*`: the app host is `*.databricksapps.com`, not the profile host. `TODO(#158)`.
   - `knowledge.request_publish`: no publish endpoint is documented, and B3 permits DRAFT Pages only. `TODO(#219)`.

   `genie.download_viz` is wired, but its reply is a PNG. Rule 6 refuses it with `non_json_reply` until a binary reply type exists through the contract owner.
6. **Host policy seam.** The task asked for a narrow `HostPolicy` trait if #256 is not merged. Main already has the `egress_policy::TokenEgressPolicy` seam (#224) and its contract suite for rules 2, 3 and 6. R7 uses that seam, so we did not add a second trait. `http_transport::host_policy::AllowlistEgressPolicy` is the small production policy. It is marked `TODO(#207)`, and the `http-policy` adapter replaces it. It passes `contract_violations`. It refuses IP hosts, `localhost`, plain HTTP, ports other than 443, user info, `accounts` and `accounts-*` hosts, and hosts outside the Databricks domain list. It has no loopback exception. Tests bring their own loopback policy double.
7. **Profile hosts.** `ProfileHosts::workspace_url(profile)` is the seam. `StubProfileHosts` returns `NotImplemented` (`TODO(#17)`). The URL must be a bare `scheme://host[:port]`. Else R7 gives `profile_host_invalid`.
8. **Token.** R7 gets the token through `TokenBroker::with_token` for each send. It builds `Authorization: Bearer ...` inside the closure, zeroes its own copy, and marks the header value sensitive. The header copy inside reqwest cannot be zeroed, and it drops when the send ends. R7 never logs or formats the token.
9. **Redirects.** Redirects are off in reqwest. R7 follows a 3xx only if the policy allows the token to the new URL and the redirect keeps the same host, for at most 3 hops. A 301, 302 or 303 becomes a GET without a body. R7 drops any other 3xx with `redirect_refused`.
10. **Replies.** Only a JSON content type with a JSON body reaches the caller, after `token_guard::scrub_json`. A page gives `non_json_reply`, and R7 never retries it. This applies to a 401 page too. A JSON 401 invalidates the token once and retries once. A second 401 gives `auth_expired`.
11. **Rate bucket.** The `RateBucket` seam has `reserve(profile)` and `on_rate_limited(profile, retry_after)`. `AimdBucket` is the simplest correct version: 40 calls/min in a sliding 60 s window, halved for 5 min after a 429, then 5 calls/min more each minute. `Retry-After` (seconds or IMF-fixdate) or FMAPI `error.retry_after`, clamped to 1-300 s, holds every call of the profile. A 429 returns `rate_limited` at once. The next call waits through the `Pause` seam. `TODO(#138)`: G-09b priority queues and G-10b ask backoff need the call class from the bridge.
12. **Online state.** `is_online` is `true` until a call fails to connect. A connect error sets it to `false`, and any HTTP reply sets it to `true`.
13. **Errors.** All errors are `CoreError::Unavailable { component: "R7", reason }` with a fixed code. No error text holds a URL, a host, a token or body text.

## Consequences

- SEC-06a and SEC-06b hold on the wire (loopback tests). #197 can close when this PR merges.
- The transport is blocking. Callers must run it on a worker thread, not inside an async runtime.
- The supply-chain note for #160 is in the PR: the new crates are reqwest's rustls stack and ring. There are no git dependencies.
