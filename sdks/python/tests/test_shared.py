"""What the SDK shares with the repository and its basic vocabulary: the catalog, the protocol, the errors, JSON."""

from __future__ import annotations

import json
import pickle
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

from sagat_octopus_orchestrator import (
    ERROR_CODES,
    ActivityFailure,
    SagatError,
    canonical_json,
    code_of_http_status,
    format_instant,
    is_traceparent,
    new_traceparent,
    parse_instant,
    remediation_of,
    retryable_by_default,
    trace_id_of,
)
from sagat_octopus_orchestrator._catalog import CATALOG
from sagat_octopus_orchestrator._json import decode_json, encode_json
from sagat_octopus_orchestrator.retry import RetryPolicy, backoff_delay, delay_after

PACKAGE = Path(__file__).resolve().parent.parent
ROOT = PACKAGE.parent.parent


def test_the_embedded_error_catalog_is_the_shared_one() -> None:
    shared = json.loads((ROOT / "sdks" / "error-catalog.json").read_text(encoding="utf-8"))
    embedded = [
        {
            "code": entry.code,
            "origin": entry.origin,
            "http": list(entry.http),
            "grpc": entry.grpc,
            "retryable": entry.retryable,
            "remediation": entry.remediation,
        }
        for entry in CATALOG
    ]
    assert embedded == shared["errors"], "run `python scripts/sync_shared.py`"


def test_the_catalog_module_is_what_the_sync_script_writes() -> None:
    from scripts.sync_shared import render_catalog

    shared = json.loads((ROOT / "sdks" / "error-catalog.json").read_text(encoding="utf-8"))
    module = (PACKAGE / "sagat_octopus_orchestrator" / "_catalog.py").read_text(encoding="utf-8")
    assert module == render_catalog(shared), "run `python scripts/sync_shared.py`"


def test_the_worker_protocol_shipped_with_the_package_is_the_one_of_the_contracts() -> None:
    contract = (ROOT / "specs" / "001-platform-foundation" / "contracts" / "worker.proto").read_bytes()
    assert (PACKAGE / "proto" / "worker.proto").read_bytes() == contract, "run `python scripts/sync_shared.py`"


def test_the_generated_protocol_code_is_what_the_protocol_generates() -> None:
    """The committed ``*_pb2*.py`` are what ``grpcio-tools`` makes of the shipped ``worker.proto``."""
    pytest.importorskip("grpc_tools")
    from scripts.sync_shared import GENERATED, generate_proto

    with tempfile.TemporaryDirectory() as scratch:
        generate_proto((PACKAGE / "proto" / "worker.proto").read_bytes(), Path(scratch))
        for name in GENERATED:
            committed = (PACKAGE / "sagat_octopus_orchestrator" / "_proto" / name).read_bytes()
            assert committed == (Path(scratch) / name).read_bytes(), (
                f"{name} is stale: run `python scripts/sync_shared.py`"
            )


def test_every_error_has_a_remediation_a_default_and_a_code_of_its_own() -> None:
    codes = {entry.code for entry in CATALOG}
    assert len(codes) == len(CATALOG)
    assert codes == ERROR_CODES, "the ErrorCode type and the catalog name the same codes"
    for entry in CATALOG:
        assert len(entry.remediation) > 20, entry.code
        assert remediation_of(entry.code) == entry.remediation
        assert retryable_by_default(entry.code) == entry.retryable


def test_an_error_carries_the_code_the_trace_and_the_remediation_and_says_nothing_else() -> None:
    error = SagatError("RATE_LIMITED", "slow down", status=429, trace_id="a" * 32, reason="TENANT", retry_after_ms=2000)
    assert error.retryable is True
    assert error.remediation == remediation_of("RATE_LIMITED")
    assert sorted(error.to_dict()) == [
        "code", "message", "reason", "remediation", "retryAfterMs", "retryable", "status", "traceId",
    ]  # fmt: skip
    assert str(error) == "slow down"
    restored = pickle.loads(pickle.dumps(error))
    assert restored.to_dict() == error.to_dict()


def test_a_status_without_an_envelope_is_classified_by_the_catalog() -> None:
    assert code_of_http_status(401) == "UNAUTHENTICATED"
    assert code_of_http_status(404) == "NOT_FOUND"
    assert code_of_http_status(409) == "CONFLICT"
    assert code_of_http_status(422) == "VALIDATION_FAILED"
    assert code_of_http_status(503) == "UNAVAILABLE"
    assert code_of_http_status(500) == "INTERNAL"
    assert code_of_http_status(418) == "BAD_REQUEST"


def test_an_activity_failure_says_whether_it_may_be_retried() -> None:
    assert ActivityFailure.retryable("BUSY", "later").is_retryable is True
    assert ActivityFailure.permanent("NO_FUNDS", "no").is_retryable is False
    unknown = ActivityFailure.outcome_unknown("timed out")
    assert (unknown.code, unknown.is_retryable) == ("OUTCOME_UNKNOWN", False)
    assert pickle.loads(pickle.dumps(unknown)).code == "OUTCOME_UNKNOWN"


# ------------------------------------------------------------------------------------------------- JSON


def test_integers_stay_integers_and_strings_stay_strings() -> None:
    text = encode_json({"ms": 600000, "ratio": 0.5, "amount": "19.99", "big": 2**53 - 1, "none": None, "ok": True})
    assert text == '{"ms":600000,"ratio":0.5,"amount":"19.99","big":9007199254740991,"none":null,"ok":true}'
    back = decode_json(text)
    assert type(back["ms"]) is int and type(back["ratio"]) is float and back["amount"] == "19.99"
    assert encode_json({"fraction": 0.1}) == '{"fraction":0.1}'
    assert encode_json("ação € 😀") == '"ação € 😀"'


def test_what_is_not_json_is_refused_before_it_is_sent() -> None:
    from decimal import Decimal

    for bad in (
        float("nan"),
        float("inf"),
        2**63,
        -(2**63) - 1,
        Decimal("1.5"),
        b"bytes",
        {1: "a"},
        {"a": {"b": object()}},
        "\ud800",
    ):
        with pytest.raises(SagatError) as caught:
            encode_json({"field": bad})
        assert caught.value.code == "VALIDATION_FAILED"
    cycle: list[object] = []
    cycle.append(cycle)
    with pytest.raises(SagatError):
        encode_json(cycle)
    assert encode_json(2**63 - 1) == "9223372036854775807"
    with pytest.raises(ValueError):
        decode_json("[NaN]")


def test_the_canonical_text_sorts_keys_at_every_depth_and_leaves_out_whitespace() -> None:
    assert (
        canonical_json({"b": 1, "a": {"d": [2, {"z": 1, "y": 2}], "c": None}})
        == '{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"b":1}'
    )
    assert canonical_json({"s": 'é\n"', "t": [True, False]}) == '{"s":"é\\n\\"","t":[true,false]}'


def test_keys_sort_by_utf16_code_units_not_by_code_points() -> None:
    # U+1F600 is the surrogate pair D83D DE00 in UTF-16, so it sorts BEFORE U+FF5E (code-point order: after).
    emoji, fullwidth = "\U0001f600", "\uff5e"
    assert emoji > fullwidth, "by code point the emoji is last"
    assert canonical_json({fullwidth: 1, emoji: 2, "a": 3}) == f'{{"a":3,"{emoji}":2,"{fullwidth}":1}}'
    assert canonical_json({"b": {fullwidth: 1, emoji: 2}}) == f'{{"b":{{"{emoji}":2,"{fullwidth}":1}}}}'
    from sagat_octopus_orchestrator.workflow import workflow

    text = workflow("utf-flow").step("a", "act.a").input_schema({"properties": {fullwidth: {}, emoji: {}}}).to_json()
    assert text.index(emoji) < text.index(fullwidth)
    with pytest.raises(SagatError):
        canonical_json({"\ud800": 1})


def test_numbers_in_the_canonical_text_are_written_as_javascript_writes_them() -> None:
    cases = {
        0.1: "0.1", 1.0: "1", -2.5: "-2.5", 100.0: "100", 1e21: "1e+21", 1e20: "100000000000000000000", 1e-6: "0.000001",
        1e-7: "1e-7", 1.5e300: "1.5e+300", 5e-324: "5e-324", 123456.789: "123456.789", -0.0: "0", 0.5: "0.5", 1.2345e-7: "1.2345e-7",
    }  # fmt: skip
    for value, text in cases.items():
        assert canonical_json(value) == text, value
    assert canonical_json({"a": 600000, "b": 600000.0}) == '{"a":600000,"b":600000}'


# ------------------------------------------------------------------------------------------------ instants


def test_an_instant_is_written_in_utc_with_a_z_whatever_the_offset_it_came_in() -> None:
    plus = timezone(timedelta(hours=5, minutes=30))
    assert format_instant(datetime(2030, 1, 1, 5, 30, tzinfo=plus)) == "2030-01-01T00:00:00.000Z"
    assert format_instant(datetime(2030, 1, 1, 0, 0, 0, 123456, tzinfo=timezone.utc)) == "2030-01-01T00:00:00.123Z"
    with pytest.raises(SagatError) as caught:
        format_instant(datetime(2030, 1, 1))
    assert caught.value.code == "VALIDATION_FAILED"


def test_an_instant_is_read_with_any_offset_and_returned_in_utc() -> None:
    for text in (
        "2026-10-10T10:00:00Z",
        "2026-10-10T15:30:00+05:30",
        "2026-10-10t10:00:00.000000000Z",
        "2026-10-10T04:30:00-05:30",
    ):
        assert parse_instant(text) == datetime(2026, 10, 10, 10, 0, tzinfo=timezone.utc), text
    assert parse_instant("2026-10-10T10:00:00.123456789Z") == datetime(
        2026, 10, 10, 10, 0, 0, 123456, tzinfo=timezone.utc
    )
    assert (
        parse_instant("yesterday") is None
        and parse_instant(None) is None
        and parse_instant("2026-13-40T00:00:00Z") is None
    )
    parsed = parse_instant("2026-10-10T15:30:00+05:30")
    assert parsed is not None and parsed.utcoffset() == timedelta(0)


# ------------------------------------------------------------------------------------------ trace, retry


def test_a_traceparent_is_validated_and_its_trace_id_read() -> None:
    good = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
    assert is_traceparent(good) and trace_id_of(good) == "0af7651916cd43dd8448eb211c80319c"
    for bad in (
        "",
        "not-a-trace",
        good.upper(),
        "00-" + "0" * 32 + "-b7ad6b7169203331-01",
        "01-" + good[3:],
        good + "-x",
    ):
        assert not is_traceparent(bad), bad
        assert trace_id_of(bad) is None
    assert is_traceparent(new_traceparent())


def test_the_backoff_doubles_up_to_its_cap_with_a_jitter_of_half_to_whole_and_honours_retry_after() -> None:
    policy = RetryPolicy(max_attempts=4, initial_delay=0.2, max_delay=1.0)
    assert backoff_delay(policy, 0, lambda: 1.0) == pytest.approx(0.2)
    assert backoff_delay(policy, 1, lambda: 0.0) == pytest.approx(0.2)  # 0.4 scaled by 0.5
    assert backoff_delay(policy, 5, lambda: 1.0) == pytest.approx(1.0)  # capped
    error = SagatError("RATE_LIMITED", "slow", retry_after_ms=3000)
    assert delay_after(policy, 0, error, lambda: 1.0) == pytest.approx(3.0)
    assert delay_after(policy, 0, SagatError("TRANSPORT", "x"), lambda: 1.0) == pytest.approx(0.2)
