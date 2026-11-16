"""[SP-07] Offline tests for spikes/sp-07/run.py (fake transport, no network)."""
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FAKE_HOST, FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r7 = load_run("sp-07", "sp07_run")


def _h2(lat, errors, n):
    """Evaluate with every call on HTTP/2."""
    return r7.evaluate_one(lat, errors, n, {"HTTP/2"})




def test_sp_07_models_are_system_ai_names():
    """[SP-07] [NFR-21a] The three models use system.ai.* names."""
    assert r7.MODELS == ["system.ai.meta-llama-3-1-8b-instruct", "system.ai.gpt-oss-20b",
                         "system.ai.gemini-3-1-flash-lite"]
    assert r7.PATH == "/ai-gateway/mlflow/v1/chat/completions"


def test_sp_07_requests_are_non_streaming_and_router_is_strict():
    """[SP-07] Both kinds set stream=false; the router sends the strict Appendix D schema."""
    schema = r7.load_router_schema()
    c = r7.request("cleanup", "m")
    r = r7.request("router", "m", schema)
    assert c["stream"] is False and r["stream"] is False
    assert r["response_format"]["json_schema"]["strict"] is True
    assert r["response_format"]["json_schema"]["name"] == "mog_router_decision"


def test_sp_07_measure_drops_warmup_and_errors():
    """[SP-07] Warm-up calls are not measured; non-2xx calls count as errors, not latency."""
    answers = [resp(200, {}, elapsed_ms=900)] * 2 + [resp(200, {}, elapsed_ms=100), resp(429, {"error_code": "RATE"}),
                                                    resp(200, {}, elapsed_ms=300)]
    api = sl.Api(FAKE_HOST, sl.Token(fake_token()), FakeTransport(list(answers)))
    lat, errs, ver = r7.measure(api, {"x": 1}, n=3, warmup=2)
    assert lat == [100, 300] and len(errs) == 1 and errs[0]["status"] == 429
    assert ver == {"HTTP/2"}


def test_sp_07_http1_fails_even_when_fast():
    """[SP-07] A run that did not negotiate HTTP/2 fails the criterion "on warm HTTP/2"."""
    ev = r7.evaluate_one([100] * 200, [], 200, {"HTTP/1.1"})
    assert not ev["passed"] and ev["http2"] is False and ev["http_versions"] == ["HTTP/1.1"]
    assert not r7.evaluate_one([100] * 200, [], 200, {"HTTP/1.1", "HTTP/2"})["passed"]
    assert r7.evaluate_one([100] * 200, [], 200, {"HTTP/2"})["passed"]


def test_sp_07_pass_rule_p95_at_most_800():
    """[SP-07] p95 <= 800 ms passes; 801 fails."""
    ok = _h2([500] * 180 + [800] * 20, [], 200)
    bad = _h2([500] * 180 + [801] * 20, [], 200)
    assert ok["passed"] and ok["p95_ms"] == 800
    assert not bad["passed"]


def test_sp_07_too_many_errors_fails():
    """[SP-07] Fewer than 95 % good calls is not enough evidence and fails."""
    ev = _h2([100] * 180, [{}] * 20, 200)
    assert not ev["enough_samples"] and not ev["passed"]


def test_sp_07_overall_names_fallback_and_fastest():
    """[SP-07] One slow model fails the spike; the report names the faster router model."""
    res = {"router": {"a": _h2([300] * 200, [], 200), "b": _h2([900] * 200, [], 200)},
           "cleanup": {"a": _h2([200] * 200, [], 200)}}
    ev = r7.evaluate(res)
    assert not ev["passed"] and ev["failing"] == ["router:b"]
    assert ev["fastest_by_kind"]["router"] == "a" and "(a)" in ev["fallback"]
    res["router"].pop("b")
    assert r7.evaluate(res)["passed"]


def test_sp_07_utterance_is_synthetic():
    """[SP-07] The prompt is fixed synthetic text, so the report holds no user content."""
    body = json.dumps(r7.request("cleanup", "m"))
    assert "west region" in body
