"""[SP-12] Offline tests for spikes/sp-12/run.py (parsing, SQL text, TLS probe, pass rule)."""
import json
import os
import random
import ssl
import struct
import sys
from types import SimpleNamespace

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import load_run  # noqa: E402

import spikelib as sl  # noqa: E402

r12 = load_run("sp-12", "sp12_run")

# Recorded-style fake CLI outputs (field names from the Postgres API reference; hosts is CHECK).
BRANCHES = {"branches": [{"name": "projects/p/branches/dev", "status": {"default": False}},
                         {"name": "projects/p/branches/br-1", "status": {"default": True}}]}
ENDPOINTS = {"endpoints": [{"name": "projects/p/branches/br-1/endpoints/ro", "spec": {"endpoint_type": "ENDPOINT_TYPE_READ_ONLY"}},
                           {"name": "projects/p/branches/br-1/endpoints/rw", "spec": {"endpoint_type": "ENDPOINT_TYPE_READ_WRITE"}}]}
ENDPOINT = {"name": "projects/p/branches/br-1/endpoints/rw",
            "status": {"current_state": "ACTIVE", "hosts": {"host": "ep-abc.database.us-east-1.cloud.databricks.com"},
                       "autoscaling_limit_min_cu": 0.5}}


def test_sp_12_pick_branch_and_endpoint():
    """[SP-12] The default branch and the read-write endpoint are chosen."""
    assert r12.pick_branch(BRANCHES) == "projects/p/branches/br-1"
    assert r12.pick_branch({"branches": [{"name": "projects/p/branches/production"}, {"name": "x"}]}) == "projects/p/branches/production"
    assert r12.pick_endpoint(ENDPOINTS) == "projects/p/branches/br-1/endpoints/rw"
    with pytest.raises(sl.SpikeError):
        r12.pick_endpoint({"endpoints": []})


def test_sp_12_endpoint_host_and_state():
    """[SP-12] The host comes from status.hosts; the state from status.current_state."""
    assert r12.endpoint_host(ENDPOINT) == "ep-abc.database.us-east-1.cloud.databricks.com"
    assert r12.endpoint_host({"status": {"hosts": {"read_write_host": "h2"}}}) == "h2"
    assert r12.endpoint_state(ENDPOINT) == "ACTIVE"
    with pytest.raises(sl.SpikeError):
        r12.endpoint_host({"status": {"hosts": {}}})


def test_sp_12_cli_wrapper_adds_profile_and_redacts_errors():
    """[SP-12] Cli.json() adds --profile and -o json; a failure message is redacted."""
    calls = []

    def runner(argv, **kw):
        calls.append(argv)
        if "generate-database-credential" in argv:
            return SimpleNamespace(returncode=1, stdout="", stderr="Error: token eyJabc.def.ghi expired")
        return SimpleNamespace(returncode=0, stdout=json.dumps(ENDPOINT), stderr="")

    cli = r12.Cli("my-profile", runner=runner)
    assert cli.json("postgres", "get-endpoint", "x")["name"].endswith("/rw")
    assert calls[0][-4:] == ["--profile", "my-profile", "-o", "json"]
    with pytest.raises(sl.SpikeError) as e:
        cli.json("postgres", "generate-database-credential", "x")
    assert "eyJabc" not in str(e.value)


def test_sp_12_wait_suspended_polls_until_idle():
    """[SP-12] wait_suspended() polls get-endpoint until a suspended state."""
    states = iter(["ACTIVE", "ACTIVE", "SUSPENDED"])
    cli = SimpleNamespace(json=lambda *a: {"status": {"current_state": next(states)}})
    ok, state, _ = r12.wait_suspended(cli, "e", max_s=60, sleep=lambda s: None)
    assert ok and state == "SUSPENDED"


def test_sp_12_ssl_request_bytes():
    """[SP-12] The Postgres SSLRequest is length 8 and code 80877103."""
    assert r12.ssl_request_bytes() == struct.pack("!ii", 8, 80877103) == b"\x00\x00\x00\x08\x04\xd2\x16\x2f"


class FakeSock:
    def __init__(self, answer):
        self.answer, self.sent = answer, b""

    def sendall(self, b):
        self.sent += b

    def recv(self, n):
        return self.answer

    def close(self):
        pass


class FakeCtx:
    def __init__(self, fail=False):
        self.fail = fail

    def wrap_socket(self, sock, server_hostname):
        if self.fail:
            e = ssl.SSLCertVerificationError("bad")
            e.verify_message, e.verify_code = "unable to get local issuer certificate", 20
            raise e
        return SimpleNamespace(version=lambda: "TLSv1.3", close=lambda: None,
                               getpeercert=lambda: {"issuer": ((("commonName", "Test CA"),),), "notAfter": "x"})


def test_sp_12_tls_probe_verifies_chain():
    """[SP-12] [MEM-06] The probe sends SSLRequest, then a verified TLS handshake: chain_ok."""
    sock = FakeSock(b"S")
    out = r12.tls_probe("ep.database.cloud.databricks.com", connect=lambda *a, **k: sock, context=FakeCtx())
    assert out["chain_ok"] and out["tls_version"] == "TLSv1.3" and out["issuer"] == {"commonName": "Test CA"}
    assert sock.sent == r12.ssl_request_bytes()


def test_sp_12_tls_probe_records_chain_failure_and_refusal():
    """[SP-12] A verify error or an 'N' answer gives chain_ok False with the reason."""
    out = r12.tls_probe("h.x", connect=lambda *a, **k: FakeSock(b"S"), context=FakeCtx(fail=True))
    assert out["chain_ok"] is False and "issuer" in out["error"]
    out = r12.tls_probe("h.x", connect=lambda *a, **k: FakeSock(b"N"), context=FakeCtx())
    assert out["chain_ok"] is False and out["error"] == "server refused SSL"


def test_sp_12_sql_uses_1024_dims_halfvec_and_appendix_a_hybrid():
    """[SP-12] DDL is 1024-d for vector and halfvec; the hybrid query has the vector and lexical CTEs."""
    assert "embedding halfvec(1024)" in r12.ddl("halfvec") and "embedding vector(1024)" in r12.ddl("vector")
    h = r12.hybrid_sql("halfvec")
    for part in ("<=> %(q)s::halfvec", "LIMIT 40", "websearch_to_tsquery", "ts_rank_cd", "FULL OUTER JOIN",
                 "status = 'active'", "embedding_model = %(model)s"):
        assert part in h
    name, sql = r12.index_sql("lakebase_ann", "vector")
    assert "USING lakebase_ann (embedding vector_cosine_ops)" in sql and "build_mode = 'standard'" in sql
    assert "USING hnsw (embedding halfvec_cosine_ops) WITH (m = 16, ef_construction = 64)" in r12.index_sql("hnsw", "halfvec")[1]
    assert "%(n)s" in r12.fill_sql("vector") and "::halfvec(1024)" in r12.fill_sql("halfvec")


def test_sp_12_vector_literal_and_unit_vector():
    """[SP-12] Query vectors are 1024-d unit vectors, deterministic for a seed."""
    v = r12.unit_vector(random.Random(1))
    assert len(v) == 1024 and abs(sum(x * x for x in v) - 1) < 1e-9
    assert v == r12.unit_vector(random.Random(1))
    assert r12.vector_literal([0.5, -1]) == "[0.500000,-1.000000]"


def _report(chain=True, verify=True, colds=(400, 800), ann_vec=(True, 250), ann_half=(True, 260), hnsw=180):
    return {"tls": {"chain_ok": chain}, "login": {"verify_full_ok": verify},
            "cold": [{"ok": True, "wake_ms": c} for c in colds],
            "variants": {"lakebase_ann:vector": {"built": ann_vec[0], "hybrid": {"p95_ms": ann_vec[1]}},
                         "lakebase_ann:halfvec": {"built": ann_half[0], "hybrid": {"p95_ms": ann_half[1]}},
                         "hnsw:vector": {"built": True, "hybrid": {"p95_ms": hnsw}}}}


def test_sp_12_pass_rule():
    """[SP-12] [MEM-15b] Pass: chain verifies, cold <= 1 s, lakebase_ann 1024 builds, hybrid p95 <= 300 ms."""
    ev = r12.evaluate(_report())
    assert ev["passed"] and ev["fallback"] is None and ev["cold_wake_max_ms"] == 800


def test_sp_12_fail_rules_name_their_fallbacks():
    """[SP-12] Each failed criterion names its fallback."""
    assert "sslmode=require" in r12.evaluate(_report(chain=False))["fallback"]
    assert "sslmode=require" in r12.evaluate(_report(verify=False))["fallback"]
    assert "prewarm on key-down" in r12.evaluate(_report(colds=(400, 1001)))["fallback"]
    assert "HNSW stays" in r12.evaluate(_report(ann_half=(False, None)))["fallback"]
    assert "HNSW stays" in r12.evaluate(_report(ann_vec=(True, 301)))["fallback"]
    assert not r12.evaluate(_report(colds=()))["passed"]
