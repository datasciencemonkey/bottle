# Genie Code lockdown canaries (local only)

**Local-only release gate. CI does not run it.** It needs the real, pinned `genie` (0.5.0) on the
Mac. It makes no Databricks call and no model call, and it picks no profile: the "model" is a fake
OpenAI Responses server on `127.0.0.1`, and the auth command is `/bin/echo`.

```
uv run tools/genie-lockdown/lockdown.py            # check
uv run tools/genie-lockdown/lockdown.py --record   # (re)write the two snapshots, then review them
```

It uses the golden `config.toml` of the Rust session host
(`crates/mog-core/src/agent_run/session/golden_config.toml`) and replaces only the model host, the
auth command and the shim command. It drives `genie app-server` the way the session host does
(ephemeral thread, `approvalPolicy:"never"`) and checks: the version and SHA-256 pin; the tool list
that genie offers the model (snapshot `expected-tools-<v>.json`, no shell, web search or image
tool); an `apply_patch` canary that must create no file; no server-to-client request; remote
control disabled; `env_vars` forwarding to the `bottle` MCP server; a disk canary in GENIE_HOME and
the cwd during and after the session; an egress canary through a recording proxy; and the
`genie doctor --json` enabled-flag snapshot (`expected-doctor-<v>.json`).

Run it again on each genie version change, before you change `pin.rs`.
