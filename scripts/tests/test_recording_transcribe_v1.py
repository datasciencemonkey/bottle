"""Tests for the fixed server-side recording task script (ADR 0103, PR C).

The script runs as a one-time serverless job (`jobs/runs/submit`, `spark_python_task`). Its pure
functions (parse the ai_transcribe VARIANT, merge parts to absolute times, render markdown, map
errors, update job.json) must match the Rust warehouse path, so these tests use the same cases as
`crates/mog-core/tests/recording_cloud_pure.rs`. Spark is never imported here.

Run:  uv run --no-project --with pytest pytest scripts/tests/test_recording_transcribe_v1.py -q
"""
import importlib.util
import json

from conftest import ROOT

SCRIPT = ROOT / "crates" / "mog-core" / "assets" / "recording" / "transcribe_v1.py"


def load():
    spec = importlib.util.spec_from_file_location("transcribe_v1", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


LIVE_CELL = (
    '{"error_message":null,"metadata":{"audio_stream_count":1,"degraded":false,'
    '"input_duration_seconds":4.656,"num_chunks":1,"version":"1.0"},"response":'
    '{"duration_seconds":4.669,"segments":[{"end":4.64,"speaker_id":"1","start":0,'
    '"text":"Hello team. The launch moves to Friday. Maria will send the notes."}]}}'
)


def test_rec_script_imports_without_spark():
    m = load()
    assert m.SCRIPT_VERSION == 1
    assert "ai_transcribe" in m.TRANSCRIBE_SQL and ":path" in m.TRANSCRIBE_SQL
    assert "ai_query(:endpoint" in m.NOTES_SQL


def test_rec_script_parses_the_live_variant_and_errors():
    m = load()
    dur, segs = m.parse_transcribe(LIVE_CELL)
    assert abs(dur - 4.669) < 1e-9
    assert segs == [
        {
            "start": 0.0,
            "end": 4.64,
            "speaker_id": "1",
            "text": "Hello team. The launch moves to Friday. Maria will send the notes.",
        }
    ]
    bad = '{"error_message":"Unsupported or unrecognized audio format.","response":null}'
    try:
        m.parse_transcribe(bad)
        raise AssertionError("must raise")
    except m.StageError as e:
        assert e.kind == "transcribe_failed"
    try:
        m.parse_transcribe('{"error_message":"Input audio exceeds the maximum duration.","response":null}')
        raise AssertionError("must raise")
    except m.StageError as e:
        assert e.kind == "too_long"


def test_rec_script_merge_matches_rust():
    m = load()
    parts = [
        {
            "file": "n.part01.flac",
            "duration_s": 3300.0,
            "transcript": (3299.5, [
                {"start": 0.0, "end": 2.0, "speaker_id": "1", "text": "Hi."},
                {"start": 2.0, "end": 4.0, "speaker_id": "2", "text": "Hello."},
            ]),
        },
        {
            "file": "n.part02.flac",
            "duration_s": None,
            "transcript": (60.0, [{"start": 1.0, "end": 3.0, "speaker_id": "1", "text": "Back."}]),
        },
    ]
    infos, segs, total = m.merge_parts(parts)
    assert [p["offset_s"] for p in infos] == [0.0, 3300.0]
    assert infos[1]["duration_s"] == 60.0
    assert abs(total - 3360.0) < 1e-9
    assert [s["speaker"] for s in segs] == ["1", "2", "2.1"]
    assert segs[2]["start"] == 3301.0 and segs[2]["part"] == 2
    _, one, _ = m.merge_parts(parts[:1])
    assert one[1]["speaker"] == "2"


def test_rec_script_markdown_matches_rust():
    m = load()
    doc = {
        "name": "2026-11-19_0905",
        "segments": [
            {"start": 0.0, "end": 1.0, "speaker": "1", "text": "Hi.", "part": 1},
            {"start": 1.0, "end": 2.0, "speaker": "1", "text": "Still me.", "part": 1},
            {"start": 3723.4, "end": 3725.0, "speaker": "2", "text": "Bye.", "part": 1},
        ],
    }
    assert m.render_markdown(doc) == (
        "# Call 2026-11-19_0905\n\n**Speaker 1** [00:00:00]\nHi. Still me.\n\n"
        "**Speaker 2** [01:02:03]\nBye.\n"
    )
    assert m.fmt_hms(83.9) == "00:01:23"


def test_rec_script_error_mapping_matches_rust():
    m = load()
    assert m.map_sql_error(
        "[UNRESOLVED_ROUTINE] Cannot resolve routine `ai_transcribe` on search path", "transcribe_failed"
    ) == "ai_transcribe_disabled"
    assert m.map_sql_error(
        "[AI_FUNCTION_COMPILATION_ERROR] An error occurred while compiling the AI function: "
        "INVALID_ARGUMENT: 'x' does not exist or is not accessible.",
        "notes_failed",
    ) == "endpoint_missing"
    assert m.map_sql_error(
        "PERMISSION_DENIED: User does not have READ VOLUME on Volume a.b.c", "transcribe_failed"
    ) == "volume_permission"
    assert m.map_sql_error("boom", "notes_failed") == "notes_failed"


def test_rec_script_job_json_update(tmp_path):
    m = load()
    job = tmp_path / "n.job.json"
    job.write_text(json.dumps({"version": 1, "id": "r1", "name": "n", "status": "transcribing",
                               "error_kind": "offline", "error_stage": "transcribing"}))
    m.update_job(str(job), "done", now="2026-11-19T15:00:00Z")
    got = json.loads(job.read_text())
    assert got["status"] == "done" and got["updated_at"] == "2026-11-19T15:00:00Z"
    assert "error_kind" not in got and "error_stage" not in got
    assert got["engine"] == "job"
    m.update_job(str(job), "failed", kind="too_long", stage="transcribing", now="t2")
    got = json.loads(job.read_text())
    assert (got["status"], got["error_kind"], got["error_stage"]) == ("failed", "too_long", "transcribing")


def test_rec_script_args():
    m = load()
    a = m.parse_args(["--folder", "/Volumes/c/s/v/Bottle/Recordings/n", "--name", "n",
                      "--endpoint", "databricks-claude-sonnet-5-5", "--from-stage", "writing_notes"])
    assert a.folder.endswith("/n") and a.from_stage == "writing_notes"
