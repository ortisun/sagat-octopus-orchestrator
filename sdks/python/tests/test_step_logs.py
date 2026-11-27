"""The parts of step-log capture that need no host (design 041, ``sdks/SPEC.md`` sections 6.7 and 6.9): the sizes, the
cut, the fields, the process's secret set and its redaction."""

from __future__ import annotations

import copy
import math
import pickle
from typing import Any

import pytest

from sagat_octopus_orchestrator._proto import worker_pb2 as pb
from sagat_octopus_orchestrator.config import Secret
from sagat_octopus_orchestrator.worker import _step_logs as step_logs
from sagat_octopus_orchestrator.worker._step_logs import (
    AttemptIdentity,
    SecretSet,
    build_fields,
    build_record,
    carry_size,
    cut_utf8,
    delimited,
    request_of,
    well_formed,
)


class NoSecrets:
    def redact(self, text: str) -> str:
        return text


NONE = NoSecrets()


def fields_of(built: list[pb.StepLogField]) -> list[tuple[str, str, Any]]:
    """Each field as (key, member of its oneof, value)."""
    out = []
    for field in built:
        member = field.WhichOneof("value")
        assert member is not None
        out.append((field.key, member, getattr(field, member)))
    return out


# ------------------------------------------------------------------------------------------------ sizes


def test_the_sizes_the_sdk_counts_are_the_bytes_protobuf_writes() -> None:
    records = [
        pb.StepLogRecord(sequence=1, worker_time_epoch_ms=1_795_779_247_123, level=pb.STEP_LOG_LEVEL_INFO, message="é"),
        pb.StepLogRecord(
            sequence=2**40,
            worker_time_epoch_ms=0,
            level=pb.STEP_LOG_LEVEL_TRACE,
            fields=[
                pb.StepLogField(key="zero", int_value=0),
                pb.StepLogField(key="neg", int_value=-5),
                pb.StepLogField(key="big", int_value=2**63 - 1),
                pb.StepLogField(key="no", bool_value=False),
                pb.StepLogField(key="empty", string_value=""),
                pb.StepLogField(key="d", double_value=0.25),
                pb.StepLogField(key="", string_value="x"),
            ],
            exception="ValueError: x\n  at y",
            truncated=True,
        ),
        pb.StepLogRecord(sequence=300, worker_time_epoch_ms=5, level=pb.STEP_LOG_LEVEL_ERROR, message="m" * 8192),
    ]
    for record in records:
        assert record.ByteSize() == len(record.SerializeToString())
    sizes = [record.ByteSize() for record in records]
    carry = pb.StepLogCarry(records=records, highest_sequence=2**40)
    assert carry_size(sizes, 2**40) == len(carry.SerializeToString())
    assert carry_size([], 300) == len(pb.StepLogCarry(highest_sequence=300).SerializeToString())
    identity = AttemptIdentity(
        task_id="run-1/charge/1",
        run_id="0195b2c0-0000-7000-8000-000000000001",
        step_id="0195b2c0-0000-7000-8000-000000000002",
        attempt=3,
        generation=77,
        lease_token=Secret("lt-123"),
        instance=pb.ExecutionInstanceIdentity(
            instance_path=[
                pb.ScopePathSegment(scope_node_id="items", iteration_index=0),
                pb.ScopePathSegment(scope_node_id="inner", iteration_index=129),
            ],
            definition_node_id="each",
        ),
    )
    empty = request_of(identity, "worker-1", "session-token", [])
    full = request_of(identity, "worker-1", "session-token", records)
    assert empty.ByteSize() + sum(delimited(size) for size in sizes) == len(full.SerializeToString())
    assert (full.lease_token, full.attempt, full.generation, list(full.records)) == ("lt-123", 3, 77, records)


# ------------------------------------------------------------------------------------------------ text


def test_a_message_is_cut_at_the_last_character_boundary_under_its_ceiling_and_text_is_made_well_formed() -> None:
    cut, was_cut = cut_utf8("a" + "é" * 4500, 8192)
    assert was_cut
    assert len(cut.encode("utf-8")) == 8191
    assert cut == "a" + "é" * 4095
    assert cut_utf8("short", 8192) == ("short", False)
    assert cut_utf8("x" * 8192, 8192) == ("x" * 8192, False)
    assert well_formed("a\x00b\ud800c\udc00d😀") == "a�b�c�d😀"


def test_fields_keep_their_types_drop_a_none_write_anything_else_as_compact_json_and_name_the_non_finite() -> None:
    built, truncated = build_fields(
        {
            "text": "plain",
            "count": 42,
            "ratio": 0.25,
            "whole": 2.0,
            "flag": True,
            "nothing": None,
            "list": [1, "two", True, None, 0.5, math.nan],
            "tuple": ("a", {"k": 1.0}),
            "nan": math.nan,
            "inf": math.inf,
            "minf": -math.inf,
            "nul": "a\x00b",
            "big": 2**63 - 1,
            "small": -(2**63),
            "huge": 2**64,
            "raw": b"bytes \xff text",
            "other": {1, 2} - {1, 2},
        },
        NONE,
    )
    assert not truncated
    assert fields_of(built) == [
        ("big", "int_value", 2**63 - 1),
        ("count", "int_value", 42),
        ("flag", "bool_value", True),
        ("huge", "string_value", "18446744073709551616"),
        ("inf", "string_value", "Infinity"),
        ("list", "string_value", '[1,"two",true,null,0.5,null]'),
        ("minf", "string_value", "-Infinity"),
        ("nan", "string_value", "NaN"),
        ("nul", "string_value", "a�b"),
        ("other", "string_value", "set()"),
        ("ratio", "double_value", 0.25),
        ("raw", "string_value", "bytes � text"),
        ("small", "int_value", -(2**63)),
        ("text", "string_value", "plain"),
        ("tuple", "string_value", '["a",{"k":1}]'),
        ("whole", "double_value", 2.0),
    ]


def test_fields_over_32_keys_keep_the_first_32_in_key_order_and_over_2048_bytes_stop_before_the_one_that_passes() -> (
    None
):
    many = {f"k{n:02d}": n for n in range(40, 0, -1)}
    built, truncated = build_fields(many, NONE)
    assert truncated
    assert [field.key for field in built] == [f"k{n:02d}" for n in range(1, 33)]
    big, truncated = build_fields({"a": "x" * 1100, "b": "y" * 1100, "c": True}, NONE)
    assert truncated
    assert fields_of(big) == [("a", "string_value", "x" * 1100)]
    # Exactly 2,048 bytes is kept: {"a":"x..."} is 8 bytes of frame and 2,040 of value.
    assert build_fields({"a": "x" * 2040}, NONE)[1] is False
    assert build_fields({"a": "x" * 2041}, NONE)[1] is True
    # An escaped character counts as written: "\u0001" is six bytes of JSON.
    assert build_fields({"a": "x" * 2035 + "\x01"}, NONE)[1] is True


def test_keys_colliding_after_redaction_take_2_in_the_order_of_their_original_bytes_and_a_number_holding_a_secret_is_hidden() -> (
    None
):
    secrets = SecretSet()
    secrets.add([b"canary-key-1", b"12345678"])
    built, _ = build_fields(
        {"canary-key-1": "second", "[REDACTED]": "first", "n": 12345678, "d": 1e15, "f": 0.12345678, "ok": 7}, secrets
    )
    assert fields_of(built) == [
        ("[REDACTED]", "string_value", "first"),
        ("[REDACTED]~2", "string_value", "second"),
        ("d", "double_value", 1e15),
        ("f", "string_value", "[REDACTED]"),
        ("n", "string_value", "[REDACTED]"),
        ("ok", "int_value", 7),
    ]


def test_a_record_holds_the_formatted_message_the_exception_and_the_truncation() -> None:
    record = build_record(7, 1234, 25, "%s of %d", ("EUR", 3), {"k": "v"}, "ValueError: x", NONE)
    assert (record.sequence, record.worker_time_epoch_ms, record.level) == (7, 1234, pb.STEP_LOG_LEVEL_INFO)
    assert (record.message, record.exception, record.truncated) == ("EUR of 3", "ValueError: x", False)
    assert [
        build_record(1, 1, level, "m", (), None, "", NONE).level for level in (0, 9, 10, 19, 20, 30, 39, 40, 50)
    ] == [
        pb.STEP_LOG_LEVEL_TRACE,
        pb.STEP_LOG_LEVEL_TRACE,
        pb.STEP_LOG_LEVEL_DEBUG,
        pb.STEP_LOG_LEVEL_DEBUG,
        pb.STEP_LOG_LEVEL_INFO,
        pb.STEP_LOG_LEVEL_WARN,
        pb.STEP_LOG_LEVEL_WARN,
        pb.STEP_LOG_LEVEL_ERROR,
        pb.STEP_LOG_LEVEL_ERROR,
    ]
    long = build_record(1, 1, 20, "é" * 5000, (), None, "x" * 9000, NONE)
    assert long.truncated
    assert len(long.message.encode("utf-8")) == 8192 and len(long.exception) == 8192
    mapping = build_record(1, 1, 20, "%(a)s-%(b)s", ({"a": 1, "b": 2},), None, "", NONE)
    assert mapping.message == "1-2"
    assert build_record(1, 1, 20, "bad %d", ("x",), None, "", NONE).message == "bad %d"
    assert build_record(1, 1, 20, "", (), None, "", NONE).ByteSize() == 6, "defaults are left out of the wire"


# ------------------------------------------------------------------------------------------------ the secret set


def test_the_secret_set_redacts_by_the_rule_of_redact_longest_first_values_under_6_bytes_ignored_in_the_text_left() -> (
    None
):
    secrets = SecretSet()
    secrets.add([b"xxxxxx", b"D]yyyyy", b"short", b"abcdefgh", b"abcdef"])
    # "D]yyyyy" (7 bytes) goes before "xxxxxx" (6): the match the second replacement creates is not looked for again.
    assert secrets.redact("xxxxxxyyyyy") == "[REDACTED]yyyyy"
    assert secrets.redact("abcdefgh and abcdef and short") == "[REDACTED] and [REDACTED] and short"
    assert secrets.redact("nothing here") == "nothing here"
    assert secrets.redact("") == ""
    # Equal lengths keep the order they joined the set; a value of 6 bytes in 2 characters counts by its bytes.
    ordered = SecretSet()
    ordered.add([b"aaaaab", b"baaaaa", "éèê".encode()])
    assert ordered.redact("aaaaabaaaaa") == "[REDACTED]aaaaa"
    assert ordered.redact("xéèêx") == "x[REDACTED]x"


def test_the_secret_set_is_never_printed_pickled_or_copied() -> None:
    secrets = SecretSet()
    secrets.add([b"abcdefgh", b"xxxxxx"])
    for text in (repr(secrets), str(secrets), f"{secrets}", f"{secrets!r}"):
        assert "abcdef" not in text and "xxxxxx" not in text
    assert repr(secrets) == "SecretSet(2 values)"
    assert not hasattr(secrets, "__dict__")
    with pytest.raises(TypeError):
        pickle.dumps(secrets)
    with pytest.raises(TypeError):
        copy.copy(secrets)
    with pytest.raises(TypeError):
        copy.deepcopy(secrets)


def test_the_process_secret_set_is_reachable_from_the_module_by_no_reader_and_no_redaction_only_by_an_add() -> None:
    for name, value in vars(step_logs).items():
        assert not isinstance(value, SecretSet), f"{name} is a secret set"
        assert not isinstance(getattr(value, "__self__", None), SecretSet), f"{name} is a method of a secret set"
        for inner in getattr(value, "__dict__", {}).values():
            assert not isinstance(inner, SecretSet), f"{name} holds a secret set"
    assert not hasattr(step_logs, "PROCESS_SECRETS") and not hasattr(step_logs, "_PROCESS_SECRETS")
    assert step_logs.add_process_secrets([]) is False


def test_redaction_runs_on_the_text_as_formatted_a_secret_holding_a_nul_is_found_before_the_nul_is_replaced() -> None:
    secrets = SecretSet()
    secrets.add([b"pa\x00ssword123"])
    record = build_record(
        1,
        1,
        20,
        "x pa\x00ssword123 y",
        (),
        {"k\x00pa\x00ssword123": "pa\x00ssword123"},
        "ValueError: pa\x00ssword123",
        secrets,
    )
    assert record.message == "x [REDACTED] y"
    assert fields_of(list(record.fields)) == [("k�[REDACTED]", "string_value", "[REDACTED]")]
    assert record.exception == "ValueError: [REDACTED]"


def test_a_secret_inside_a_structure_or_a_byte_buffer_or_a_formatted_argument_is_redacted_whatever_the_escaping() -> (
    None
):
    secret = 'line"one\\two\nthree'
    secrets = SecretSet()
    secrets.add([secret.encode(), b"bytes-secret-77"])

    class Holder:
        def __init__(self) -> None:
            self.value = secret

        def __repr__(self) -> str:
            return f"Holder({self.value!r})"

    built, _ = build_fields(
        {
            "nested": {"inner": [secret, {secret: 1}], "plain": "ok"},
            "list": ["a", secret],
            "raw": b"bytes-secret-77",
            "view": memoryview(b"bytes-secret-77"),
            "array": bytearray(b"bytes-secret-77"),
            "deep": {"buffer": [b"bytes-secret-77"]},
        },
        secrets,
    )
    text = str(fields_of(built))
    assert "line" not in text and "three" not in text and "bytes-secret-77" not in text, text
    assert "Ynl0ZXMt" not in text, "never base64"
    assert fields_of(built) == [
        ("array", "string_value", "[REDACTED]"),
        ("deep", "string_value", '{"buffer":["[REDACTED]"]}'),
        ("list", "string_value", '["a","[REDACTED]"]'),
        ("nested", "string_value", '{"inner":["[REDACTED]",{"[REDACTED]":1}],"plain":"ok"}'),
        ("raw", "string_value", "[REDACTED]"),
        ("view", "string_value", "[REDACTED]"),
    ]
    # A message formatted with %r would print the secret escaped: its arguments are redacted before they are formatted.
    record = build_record(1, 1, 20, "value %r and %s", (secret, [secret]), None, "", secrets)
    assert record.message == "value '[REDACTED]' and ['[REDACTED]']"
    assert "three" not in build_record(1, 1, 20, "%(v)r", ({"v": secret},), None, "", secrets).message


def test_a_secret_that_is_not_valid_utf8_joins_the_set_as_its_lenient_text() -> None:
    secrets = SecretSet()
    raw = b"\xff" + b"s3cr3t-token-42"
    secrets.add([raw])
    assert secrets.redact(f"logged {raw.decode('utf-8', 'replace')}") == "logged [REDACTED]"


def test_the_secret_set_holds_1024_distinct_values_or_256_kib_and_the_one_that_does_not_fit_stops_capture_once() -> (
    None
):
    secrets = SecretSet()
    values = [f"secret-{n:06d}".encode() for n in range(1024)]
    assert secrets.add(values) is False
    assert secrets.add([values[0]]) is False, "a value already held takes no room"
    assert secrets.add([b"short"]) is False, "a value under 6 bytes is never held"
    assert not secrets.full
    assert secrets.add([b"one-value-too-many"]) is True
    assert secrets.full
    assert secrets.add([b"and-another-one"]) is False, "stopped once"
    by_bytes = SecretSet()
    assert by_bytes.add([b"x" * (256 * 1024)]) is False
    assert by_bytes.add([b"y" * 6]) is True, "past 256 KiB"


def test_the_prefilter_finds_every_value_wherever_it_starts() -> None:
    secrets = SecretSet()
    values = ["abcdefghijklmnop", "QRSTUV", "éàüöçñ", "0123456789abcdefghij"]
    secrets.add([value.encode() for value in values])
    for value in values:
        for offset in range(25):
            text = "-" * offset + value + "+" * (25 - offset)
            assert secrets.redact(text) == "-" * offset + "[REDACTED]" + "+" * (25 - offset), (value, offset)
    assert secrets.redact("QRSTU" + "V") == "[REDACTED]"
    assert secrets.redact("QRSTU") == "QRSTU"
