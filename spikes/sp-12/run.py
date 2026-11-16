#!/usr/bin/env python3
"""SP-12 (#191): Lakebase cold start, TLS + OAuth login, vectors and hybrid query p95.

Steps (all through `databricks postgres ... --profile <p>` and psycopg; nothing else):
 1. Create project `bottle-sp12-<suffix>` (records the create-project JSON shape) or use --project-id.
 2. Find the read-write endpoint; set it to 0.5 CU min and max and 60 s suspend (CHECK mask syntax).
 3. Credential from `generate-database-credential` (MEM-05a). It is the Postgres password and stays in memory.
 4. TLS probe (stdlib): TCP, Postgres SSLRequest, TLS handshake with the system CA store and the host
    name check. This is the `sslmode=verify-full` chain check.
 5. psycopg connect with sslmode=verify-full, sslrootcert=system: time to login, then SELECT 1.
 6. Cold wake: wait until the endpoint suspends, then time connect + SELECT 1 (--cold-runs times).
 7. Vectors (Appendix A): tables with vector(1024) and halfvec(1024); HNSW (MVP form) and
    lakebase_ann (migration 0002) indexes; hybrid query (vector CTE + lexical CTE) p50/p95, warm.
 8. Cleanup: drop schema bottle_sp12. --delete-project deletes the project only if this run made it.

Pass: warm hybrid p95 <= 300 ms; cold wake <= 1 s; 1024-d index builds; chain verifies.
Fallback: pgvector HNSW stays; sslmode=require; prewarm on key-down.

WARNING: `CREATE EXTENSION lakebase_vector` needs Lakebase Search on in the project and is an
irreversible admin step (Appendix A). Use a throwaway spike project, which is the default.

Run:
  uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> --out sp12.json
"""
import argparse
import json
import os
import random
import socket
import ssl
import struct
import subprocess
import sys
import time
import uuid

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-12"
DIM = 1024
SCHEMA = "bottle_sp12"
HYBRID_LIMIT_MS = 300.0
COLD_LIMIT_MS = 1000.0
SUSPENDED_STATES = {"SUSPENDED", "IDLE", "INACTIVE"}  # CHECK: exact enum of status.current_state
WORDS = ["revenue", "churn", "pipeline", "renewal", "pricing", "forecast", "margin", "region",
         "customer", "contract", "segment", "quarter", "partner", "launch", "discount", "budget"]
VARIANTS = [("hnsw", "vector"), ("hnsw", "halfvec"), ("lakebase_ann", "vector"), ("lakebase_ann", "halfvec")]


# ------------------------------------------------------------------ CLI helpers (pure parsing)


class Cli:
    def __init__(self, profile, runner=subprocess.run):
        self.profile = profile
        self.runner = runner

    def json(self, *args, timeout=900):
        res = sl.run_cli([*args, "--profile", self.profile, "-o", "json"], runner=self.runner, timeout=timeout)
        if not res.ok:
            raise sl.SpikeError(f"databricks {' '.join(args[:2])} failed: " + sl.redact(res.stderr.strip())[:300])
        return res.json()


def items(listing, key):
    if isinstance(listing, list):
        return listing
    return (listing or {}).get(key, [])


def pick_branch(branches):
    """The default branch: status.default true, else name ending in /production, else the first."""
    bs = items(branches, "branches")
    for b in bs:
        if (b.get("status") or {}).get("default") is True:
            return b["name"]
    for b in bs:
        if b.get("name", "").endswith("/production"):
            return b["name"]
    if not bs:
        raise sl.SpikeError("project has no branch")
    return bs[0]["name"]


def pick_endpoint(endpoints):
    eps = items(endpoints, "endpoints")
    for e in eps:
        t = (e.get("spec") or {}).get("endpoint_type") or (e.get("status") or {}).get("endpoint_type")
        if t == "ENDPOINT_TYPE_READ_WRITE":
            return e["name"]
    if not eps:
        raise sl.SpikeError("branch has no endpoint")
    return eps[0]["name"]


def endpoint_host(ep):
    """status.hosts.host (CHECK: the reference page shows hosts only as {})."""
    hosts = (ep.get("status") or {}).get("hosts") or {}
    for k in ("host", "read_write_host", "rw_host"):
        if isinstance(hosts, dict) and hosts.get(k):
            return hosts[k]
    found = sl.find_key(hosts, "host")
    if not found:
        raise sl.SpikeError("no host in get-endpoint output (status.hosts)")
    return found


def endpoint_state(ep):
    return (ep.get("status") or {}).get("current_state")


# ------------------------------------------------------------------ TLS probe (verify-full chain)


def ssl_request_bytes():
    """Postgres SSLRequest: Int32 length 8, Int32 code 80877103."""
    return struct.pack("!ii", 8, 80877103)


def tls_probe(host, port=5432, timeout=5.0, connect=socket.create_connection, context=None):
    out = {"host_suffix": host.split(".", 1)[-1] if "." in host else host}
    t0 = time.perf_counter()
    sock = connect((host, port), timeout=timeout)
    out["tcp_ms"] = round((time.perf_counter() - t0) * 1000, 1)
    try:
        sock.sendall(ssl_request_bytes())
        answer = sock.recv(1)
        out["ssl_request_answer"] = answer.decode("ascii", "replace")
        if answer != b"S":
            out["chain_ok"] = False
            out["error"] = "server refused SSL"
            return out
        ctx = context or ssl.create_default_context()
        t1 = time.perf_counter()
        try:
            tls = ctx.wrap_socket(sock, server_hostname=host)
        except ssl.SSLCertVerificationError as e:
            out["tls_ms"] = round((time.perf_counter() - t1) * 1000, 1)
            out["chain_ok"] = False
            out["error"] = f"{e.verify_message} (code {e.verify_code})"
            return out
        out["tls_ms"] = round((time.perf_counter() - t1) * 1000, 1)
        out["chain_ok"] = True
        out["tls_version"] = tls.version()
        cert = tls.getpeercert() or {}
        out["issuer"] = dict(x[0] for x in cert.get("issuer", ()))
        out["not_after"] = cert.get("notAfter")
        tls.close()
    finally:
        try:
            sock.close()
        except OSError:
            pass
    return out


# ------------------------------------------------------------------ SQL


def vector_literal(vec):
    return "[" + ",".join(f"{x:.6f}" for x in vec) + "]"


def unit_vector(rng, dim=DIM):
    v = [rng.gauss(0, 1) for _ in range(dim)]
    n = sum(x * x for x in v) ** 0.5 or 1.0
    return [x / n for x in v]


def table_name(kind):
    return f"{SCHEMA}.items_{kind}"


def ddl(kind):
    """Appendix A subset of memory_item for one vector type ('vector' or 'halfvec')."""
    return (f"CREATE TABLE IF NOT EXISTS {table_name(kind)} ("
            "item_id uuid PRIMARY KEY, mog_id uuid NOT NULL, status text NOT NULL DEFAULT 'active', "
            "embedding_model text NOT NULL, text text NOT NULL, "
            f"embedding {kind}({DIM}), "
            "text_tsv tsvector GENERATED ALWAYS AS (to_tsvector('english', text)) STORED)")


def fill_sql(kind):
    """Server-side rows: random 1024-d vectors and 12-word texts. Params: n, mog_id, model."""
    words = "ARRAY[" + ",".join(f"'{w}'" for w in WORDS) + "]"
    return (f"INSERT INTO {table_name(kind)} (item_id, mog_id, embedding_model, text, embedding) "
            "SELECT gen_random_uuid(), %(mog)s::uuid, %(model)s, "
            f"array_to_string(ARRAY(SELECT ({words})[1 + floor(random() * {len(WORDS)})::int] "
            "FROM generate_series(1, 12) WHERE g > 0), ' '), "
            f"(SELECT array_agg(random() - 0.5 + g * 0) FROM generate_series(1, {DIM}))::{kind}({DIM}) "
            "FROM generate_series(1, %(n)s) AS g")


def index_sql(method, kind):
    ops = f"{kind}_cosine_ops"  # CHECK: halfvec_cosine_ops for lakebase_ann is not in the docs page
    name = f"items_{kind}_{method}"
    if method == "hnsw":
        return name, f"CREATE INDEX {name} ON {table_name(kind)} USING hnsw (embedding {ops}) WITH (m = 16, ef_construction = 64)"
    return name, f"CREATE INDEX {name} ON {table_name(kind)} USING lakebase_ann (embedding {ops}) WITH (build_mode = 'standard')"


def hybrid_sql(kind):
    """Appendix A mem.recall.hybrid: vector CTE + lexical CTE, one round trip."""
    t = table_name(kind)
    return (
        f"WITH v AS (SELECT item_id, embedding <=> %(q)s::{kind} AS dist FROM {t} "
        "WHERE mog_id = %(mog)s::uuid AND status = 'active' AND embedding_model = %(model)s "
        f"ORDER BY embedding <=> %(q)s::{kind} LIMIT 40), "
        "l AS (SELECT item_id, ts_rank_cd(text_tsv, websearch_to_tsquery('english', %(qt)s)) AS rank "
        f"FROM {t} WHERE mog_id = %(mog)s::uuid AND status = 'active' "
        "AND text_tsv @@ websearch_to_tsquery('english', %(qt)s) ORDER BY rank DESC LIMIT 40) "
        "SELECT item_id, v.dist, l.rank FROM v FULL OUTER JOIN l USING (item_id)")


# ------------------------------------------------------------------ evaluation


def evaluate(r):
    chain = (r.get("tls") or {}).get("chain_ok") is True and (r.get("login") or {}).get("verify_full_ok") is True
    colds = [c["wake_ms"] for c in r.get("cold", []) if c.get("ok")]
    cold_ok = bool(colds) and max(colds) <= COLD_LIMIT_MS
    v = r.get("variants", {})
    ann_vec, ann_half = v.get("lakebase_ann:vector", {}), v.get("lakebase_ann:halfvec", {})
    builds = ann_vec.get("built") is True and ann_half.get("built") is True
    p95 = (ann_vec.get("hybrid") or {}).get("p95_ms")
    hybrid_ok = p95 is not None and p95 <= HYBRID_LIMIT_MS
    passed = chain and cold_ok and builds and hybrid_ok
    fb = []
    if not (builds and hybrid_ok):
        fb.append("pgvector HNSW stays")
    if not chain:
        fb.append("sslmode=require")
    if not cold_ok:
        fb.append("prewarm on key-down")
    return {"passed": passed, "chain_verifies": chain, "cold_wake_max_ms": max(colds) if colds else None,
            "cold_ok": cold_ok, "lakebase_ann_1024_builds": builds, "hybrid_p95_ms_lakebase_ann": p95,
            "hybrid_p95_ms_hnsw": (v.get("hnsw:vector", {}).get("hybrid") or {}).get("p95_ms"),
            "hybrid_ok": hybrid_ok, "fallback": "; ".join(fb) or None}


# ------------------------------------------------------------------ live steps (psycopg)


class Lakebase:
    def __init__(self, cli, endpoint, host, user):
        self.cli, self.endpoint, self.host, self.user = cli, endpoint, host, user
        self.secrets = []

    def password(self):
        cred = self.cli.json("postgres", "generate-database-credential", self.endpoint)
        tok = sl.Token(cred.get("token"))
        self.secrets.append(tok.reveal())
        return tok

    def connect(self, sslmode="verify-full", tok=None):
        import psycopg

        tok = tok or self.password()
        t0 = time.perf_counter()
        conn = psycopg.connect(host=self.host, port=5432, dbname="databricks_postgres", user=self.user,
                               password=tok.reveal(), sslmode=sslmode,
                               sslrootcert="system" if sslmode == "verify-full" else None,
                               connect_timeout=5, autocommit=True, application_name="bottle-sp12")
        ms = round((time.perf_counter() - t0) * 1000, 1)
        tok.wipe()
        return conn, ms


def timed(conn, sql, params=None):
    t0 = time.perf_counter()
    with conn.cursor() as cur:
        cur.execute(sql, params)
        if cur.description:
            cur.fetchall()
    return round((time.perf_counter() - t0) * 1000, 1)


def wait_suspended(cli, endpoint, max_s, poll_s=10, sleep=time.sleep):
    t0 = time.monotonic()
    state = None
    while time.monotonic() - t0 < max_s:
        state = endpoint_state(cli.json("postgres", "get-endpoint", endpoint))
        if state in SUSPENDED_STATES:
            return True, state, round(time.monotonic() - t0)
        sleep(poll_s)
    return False, state, round(time.monotonic() - t0)


def run_vectors(conn, rows, queries, warmup, rng):
    out = {}
    mog = str(uuid.uuid4())
    model = "databricks-gte-large-en"
    out["extensions"] = {}
    for ext, sql in (("vector", "CREATE EXTENSION IF NOT EXISTS vector"),
                     ("lakebase_vector", "CREATE EXTENSION IF NOT EXISTS lakebase_vector CASCADE"),
                     ("lakebase_text", "CREATE EXTENSION IF NOT EXISTS lakebase_text")):
        try:
            timed(conn, sql)
            out["extensions"][ext] = "ok"
        except Exception as e:  # noqa: BLE001 - record and go on
            out["extensions"][ext] = sl.redact(f"{type(e).__name__}: {e}")[:300]
    timed(conn, f"CREATE SCHEMA IF NOT EXISTS {SCHEMA}")
    variants = {}
    for kind in ("vector", "halfvec"):
        timed(conn, ddl(kind))
        out[f"fill_ms_{kind}"] = timed(conn, fill_sql(kind), {"n": rows, "mog": mog, "model": model})
        timed(conn, f"ANALYZE {table_name(kind)}")
        for method in ("hnsw", "lakebase_ann"):
            key = f"{method}:{kind}"
            name, sql = index_sql(method, kind)
            v = {"index": name}
            try:
                v["build_ms"] = timed(conn, sql)
                v["built"] = True
            except Exception as e:  # noqa: BLE001
                v["built"] = False
                v["error"] = sl.redact(f"{type(e).__name__}: {e}")[:300]
            if v["built"]:
                lat = []
                for i in range(warmup + queries):
                    p = {"q": vector_literal(unit_vector(rng)), "mog": mog, "model": model,
                         "qt": " ".join(rng.sample(WORDS, 2))}
                    ms = timed(conn, hybrid_sql(kind), p)
                    if i >= warmup:
                        lat.append(ms)
                v["hybrid"] = sl.summary(lat)
                timed(conn, f"DROP INDEX IF EXISTS {SCHEMA}.{name}")
            variants[key] = v
            sl.say(f"  {key}: built={v['built']} hybrid={v.get('hybrid', {}).get('p95_ms')}")
    out["variants"] = variants
    out["rows"] = rows
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", required=True, help="Databricks CLI profile (you choose it)")
    p.add_argument("--out", required=True)
    p.add_argument("--project-id", help="use this project instead of creating one")
    p.add_argument("--skip-resize", action="store_true", help="do not change CU or suspend settings")
    p.add_argument("--cold-runs", type=int, default=3)
    p.add_argument("--suspend-wait-s", type=int, default=420)
    p.add_argument("--rows", type=int, default=5000)
    p.add_argument("--queries", type=int, default=200)
    p.add_argument("--warmup", type=int, default=10)
    p.add_argument("--keep-schema", action="store_true")
    p.add_argument("--delete-project", action="store_true", help="delete the project at the end if this run made it")
    args = p.parse_args(argv)
    cli = Cli(args.profile)
    rep = {"spike": SPIKE, "issue": 191, "created_by_script": False}
    lb = None

    def go():
        nonlocal lb
        if args.project_id:
            pid = args.project_id
        else:
            pid = f"bottle-sp12-{uuid.uuid4().hex[:6]}"
            created = cli.json("postgres", "create-project", pid, "--json",
                               json.dumps({"spec": {"display_name": "Bottle SP-12 spike"}}))
            rep["created_by_script"] = True
            rep["create_project_shape"] = sl.json_shape(created)
        rep["project_id"] = pid
        branch = pick_branch(cli.json("postgres", "list-branches", f"projects/{pid}"))
        endpoint = pick_endpoint(cli.json("postgres", "list-endpoints", branch))
        if not args.skip_resize:
            spec = {"spec": {"autoscaling_limit_min_cu": 0.5, "autoscaling_limit_max_cu": 0.5,
                             "suspend_timeout_duration": "60s"}}
            cli.json("postgres", "update-endpoint", endpoint,
                     "spec.autoscaling_limit_min_cu,spec.autoscaling_limit_max_cu,spec.suspend_timeout_duration",
                     "--json", json.dumps(spec))
        ep = cli.json("postgres", "get-endpoint", endpoint)
        rep["endpoint_shape"] = sl.json_shape(ep)
        rep["endpoint_cu"] = {k: (ep.get("status") or {}).get(k) for k in
                              ("autoscaling_limit_min_cu", "autoscaling_limit_max_cu", "suspend_timeout_duration")}
        host = endpoint_host(ep)
        user = cli.json("current-user", "me").get("userName")
        lb = Lakebase(cli, endpoint, host, user)
        sl.say(f"{SPIKE}: project={pid} endpoint state={endpoint_state(ep)}")
        rep["tls"] = tls_probe(host)
        login = {}
        try:
            conn, ms = lb.connect("verify-full")
            login.update(verify_full_ok=True, connect_ms=ms, select1_ms=timed(conn, "SELECT 1"))
        except Exception as e:  # noqa: BLE001
            login.update(verify_full_ok=False, error=sl.redact(f"{type(e).__name__}: {e}", lb.secrets)[:300])
            conn, ms = lb.connect("require")
            login.update(require_connect_ms=ms)
        login["note"] = "connect_ms = TCP + TLS + OAuth login; tls.tcp_ms and tls.tls_ms split it"
        rep["login"] = login
        rep["warm_connects_ms"] = []
        for _ in range(5):
            c2, ms = lb.connect("verify-full" if login["verify_full_ok"] else "require")
            rep["warm_connects_ms"].append(ms)
            c2.close()
        rep.update(run_vectors(conn, args.rows, args.queries, args.warmup, random.Random(1201)))
        if not args.keep_schema:
            timed(conn, f"DROP SCHEMA IF EXISTS {SCHEMA} CASCADE")
            rep["schema_dropped"] = True
        conn.close()
        rep["cold"] = []
        for i in range(args.cold_runs):
            ok, state, waited = wait_suspended(cli, endpoint, args.suspend_wait_s)
            row = {"run": i + 1, "suspended": ok, "state_seen": state, "waited_s": waited}
            if ok:
                tok = lb.password()  # fetched before the clock starts, as LakebasePool holds one
                t0 = time.perf_counter()
                c3, cms = lb.connect("verify-full" if login["verify_full_ok"] else "require", tok=tok)
                timed(c3, "SELECT 1")
                row.update(ok=True, wake_ms=round((time.perf_counter() - t0) * 1000, 1), connect_ms=cms)
                c3.close()
            else:
                row["ok"] = False
            rep["cold"].append(row)
            sl.say(f"  cold run {i + 1}: {row}")
        rep["evaluation"] = evaluate(rep)
        if args.delete_project and rep["created_by_script"]:
            sl.run_cli(["postgres", "delete-project", f"projects/{pid}", "--profile", args.profile])
            rep["project_deleted"] = True
        ev = rep["evaluation"]
        sl.write_report(args.out, rep, secrets=lb.secrets)
        sl.say(sl.verdict(SPIKE, ev["passed"],
                          f"chain={ev['chain_verifies']} cold_max={ev['cold_wake_max_ms']} "
                          f"ann_builds={ev['lakebase_ann_1024_builds']} hybrid_p95={ev['hybrid_p95_ms_lakebase_ann']} "
                          f"(hnsw {ev['hybrid_p95_ms_hnsw']})"))
        return 0 if ev["passed"] else 1

    code = sl.main_guard(go)
    if code and code >= 2:
        rep["aborted"] = True
        sl.write_report(args.out, rep, secrets=lb.secrets if lb else [])
        sl.say(f"{SPIKE}: partial report written; project_id={rep.get('project_id')} "
               f"created_by_script={rep['created_by_script']} (see RUN.md cleanup)")
    return code


if __name__ == "__main__":
    sys.exit(main())
