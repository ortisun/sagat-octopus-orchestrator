"""The lossless mapping of a record to the columns of a table (feature 035: T035-049, FR-035-010, D035-091).

A value is stored as it is or the write is refused: nothing is rounded, truncated, wrapped, coerced or given a zone it
did not have. A refusal names the column and a reason from a closed set, never the value (``failures.py`` has no free
text).
"""

from __future__ import annotations

import datetime
import signal
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from decimal import Decimal

import pytest
from pyiceberg.schema import Schema
from pyiceberg.types import (
    BinaryType,
    BooleanType,
    DateType,
    DecimalType,
    DoubleType,
    FixedType,
    FloatType,
    IcebergType,
    IntegerType,
    ListType,
    LongType,
    MapType,
    NestedField,
    StringType,
    StructType,
    TimestampType,
    TimestamptzType,
    TimeType,
    UUIDType,
)

from sagat_lake.failures import Failure
from sagat_lake.mapping import map_record

UTC = datetime.UTC


def schema(*fields: NestedField) -> Schema:
    return Schema(*fields)


def column(name: str, kind: IcebergType, *, required: bool = False, field_id: int = 1) -> NestedField:
    return NestedField(field_id=field_id, name=name, field_type=kind, required=required)


def one(kind: IcebergType, value: object, *, required: bool = False) -> object:
    """The stored value of a record that holds only ``value`` in the column ``x``."""
    row, dropped = map_record(schema(column("x", kind, required=required)), {"x": value})
    assert dropped == 0
    return row["x"]


def refused(kind: IcebergType, value: object, reason: str, *, name: str = "x", required: bool = False) -> Failure:
    with pytest.raises(Failure) as caught:
        map_record(schema(column(name, kind, required=required)), {name: value})
    failure = caught.value
    assert failure.code == "ICEBERG_SCHEMA_MISMATCH"
    assert failure.reason == reason
    assert failure.column == name
    return failure


# ---------------------------------------------------------------------------- what is stored as it is
def test_scalars_are_stored_as_they_are() -> None:
    assert one(BooleanType(), True) is True
    assert one(StringType(), "olá") == "olá"
    assert one(IntegerType(), 42) == 42
    assert one(LongType(), 9_007_199_254_740_993) == 9_007_199_254_740_993
    assert one(DateType(), "2026-11-06") == datetime.date(2026, 11, 6)
    assert one(UUIDType(), "8f1c2a3b-0000-4000-8000-000000000001") == uuid.UUID("8f1c2a3b-0000-4000-8000-000000000001")


def test_a_large_integer_may_arrive_as_a_decimal_string() -> None:
    # The five-language representation (FR-017-016): a JSON number cannot hold 2^63 - 1 in every language.
    assert one(LongType(), "9223372036854775807") == 9_223_372_036_854_775_807


def test_decimals_arrive_as_strings_or_exact_numbers_and_keep_their_digits() -> None:
    assert one(DecimalType(12, 4), "12345678.1230") == Decimal("12345678.1230")
    assert one(DecimalType(12, 2), Decimal("12.50")) == Decimal("12.50")
    # a trailing zero beyond the scale carries no information, so nothing is lost
    assert one(DecimalType(12, 2), "12.500") == Decimal("12.50")
    assert one(DecimalType(12, 2), 7) == Decimal("7.00")


def test_floating_columns_take_finite_numbers_and_also_nan() -> None:
    assert one(DoubleType(), 1.5) == 1.5
    assert one(DoubleType(), 3) == 3.0
    nan = one(DoubleType(), float("nan"))
    assert nan != nan  # NaN is a value a floating column can hold
    assert one(FloatType(), float("inf")) == float("inf")


def test_timestamps_keep_the_zone_they_have() -> None:
    assert one(TimestampType(), "2026-11-06T10:15:30.123456") == datetime.datetime(2026, 11, 6, 10, 15, 30, 123456)
    assert one(TimestamptzType(), "2026-11-06T10:15:30+02:00") == datetime.datetime(2026, 11, 6, 8, 15, 30, tzinfo=UTC)
    assert one(TimestamptzType(), "2026-11-06T10:15:30Z") == datetime.datetime(2026, 11, 6, 10, 15, 30, tzinfo=UTC)
    # equal instants compare equal whatever their offset, so the stored offset is checked on its own
    stored = one(TimestamptzType(), "2026-11-06T10:15:30+02:00")
    assert isinstance(stored, datetime.datetime)
    assert stored.utcoffset() == datetime.timedelta(0)
    assert one(TimeType(), "10:15:30.5") == datetime.time(10, 15, 30, 500000)


def test_binary_arrives_as_the_typed_cell_with_padded_base64() -> None:
    assert one(BinaryType(), {"type": "binary", "value": "AQID"}) == b"\x01\x02\x03"
    assert one(FixedType(3), {"type": "binary", "value": "AQID"}) == b"\x01\x02\x03"


# ---------------------------------------------------------------------------- what is refused
@pytest.mark.parametrize(
    ("kind", "value", "reason"),
    [
        # decimal scale overflow: a digit would be rounded away
        (DecimalType(12, 2), "12.505", "SCALE_OVERFLOW"),
        # decimal precision overflow: too many digits before the point
        (DecimalType(5, 2), "12345.00", "OVERFLOW"),
        # integer overflow, at both edges and for both widths
        (IntegerType(), 2_147_483_648, "OVERFLOW"),
        (IntegerType(), -2_147_483_649, "OVERFLOW"),
        (LongType(), "9223372036854775808", "OVERFLOW"),
        (LongType(), -9_223_372_036_854_775_809, "OVERFLOW"),
        # a timestamp without a zone into a zoned column: the zone is not guessed
        (TimestamptzType(), "2026-11-06T10:15:30", "ZONE"),
        # and a zone into a column that has none: it would be dropped
        (TimestampType(), "2026-11-06T10:15:30+02:00", "ZONE"),
        (TimestampType(), "2026-11-06T10:15:30Z", "ZONE"),
        (TimeType(), "10:15:30+02:00", "ZONE"),
        # sub-microsecond digits would be truncated
        (TimestampType(), "2026-11-06T10:15:30.1234567", "SCALE_OVERFLOW"),
        # NaN and infinities are for floating columns only
        (IntegerType(), float("nan"), "TYPE"),
        (LongType(), float("inf"), "TYPE"),
        (DecimalType(12, 2), float("nan"), "TYPE"),
        # a string into binary: binary has its own representation
        (BinaryType(), "AQID", "TYPE"),
        (FixedType(3), "AQID", "TYPE"),
        # a fixed value of the wrong length is not padded or cut
        (FixedType(4), {"type": "binary", "value": "AQID"}, "TYPE"),
        # a fraction into an integer, text into a number, a number into text
        (IntegerType(), 1.5, "TYPE"),
        (IntegerType(), "12x", "TYPE"),
        (LongType(), True, "TYPE"),
        (BooleanType(), 1, "TYPE"),
        (StringType(), 12, "TYPE"),
        (DateType(), "2026-13-45", "TYPE"),
        (UUIDType(), "not-a-uuid", "TYPE"),
        (DecimalType(12, 2), "twelve", "TYPE"),
        # an integer is digits and nothing else: no spaces, sign, separator or digits of another script
        (IntegerType(), " 12", "TYPE"),
        (IntegerType(), "12 ", "TYPE"),
        (IntegerType(), "+12", "TYPE"),
        (IntegerType(), "1_000", "TYPE"),
        (IntegerType(), "\u0661\u0662", "TYPE"),
        (IntegerType(), "", "TYPE"),
        # a float is already inexact, so it is not a decimal
        (DecimalType(12, 2), 12.5, "TYPE"),
        # base64 that is not padded, or that has characters outside the alphabet, is not read charitably
        (BinaryType(), {"type": "binary", "value": "AQI"}, "TYPE"),
        (BinaryType(), {"type": "binary", "value": "A Q I D"}, "TYPE"),
        (BinaryType(), {"type": "binary", "value": "AQ!D"}, "TYPE"),
    ],
)
def test_a_value_the_column_cannot_hold_is_refused_by_name_and_reason(
    kind: IcebergType, value: object, reason: str
) -> None:
    refused(kind, value, reason)


def test_a_double_that_is_not_a_float_is_not_squeezed_into_one() -> None:
    # 1e39 is finite as a double and past the largest float32
    refused(FloatType(), 1e39, "OVERFLOW")


def test_a_refusal_never_carries_the_value() -> None:
    secret = "4111-1111-1111-1111-canary"
    failure = refused(IntegerType(), secret, "TYPE")
    assert secret not in repr(failure.as_member())
    assert secret not in str(failure)


# ---------------------------------------------------------------------------- nulls and absent columns
def test_a_missing_optional_column_becomes_null() -> None:
    row, dropped = map_record(
        schema(column("a", StringType(), field_id=1), column("b", StringType(), field_id=2)), {"a": "x"}
    )
    assert row == {"a": "x", "b": None}
    assert dropped == 0


def test_an_explicit_null_into_an_optional_column_is_null() -> None:
    assert one(StringType(), None) is None


def test_a_missing_required_column_is_refused() -> None:
    with pytest.raises(Failure) as caught:
        map_record(schema(column("a", StringType(), required=True)), {})
    assert (caught.value.code, caught.value.column, caught.value.reason) == (
        "ICEBERG_SCHEMA_MISMATCH",
        "a",
        "MISSING_REQUIRED",
    )


def test_a_null_into_a_required_column_is_refused() -> None:
    refused(StringType(), None, "NULL_IN_REQUIRED", required=True)


# ---------------------------------------------------------------------------- unknown fields
def test_a_record_field_with_no_column_is_refused() -> None:
    with pytest.raises(Failure) as caught:
        map_record(schema(column("a", StringType())), {"a": "x", "ghost": 1})
    assert (caught.value.code, caught.value.column, caught.value.reason) == (
        "ICEBERG_SCHEMA_MISMATCH",
        "ghost",
        "UNKNOWN_FIELD",
    )


def test_an_unknown_field_is_dropped_and_counted_when_the_step_says_so() -> None:
    row, dropped = map_record(
        schema(column("a", StringType())), {"a": "x", "ghost": 1, "other": 2}, ignore_unknown=True
    )
    assert row == {"a": "x"}
    assert dropped == 2


def test_an_unknown_field_is_dropped_before_any_check() -> None:
    # `ghost` would not map to anything, and must not be looked at, let alone refused for its value
    row, dropped = map_record(schema(column("a", IntegerType())), {"a": 1, "ghost": float("nan")}, ignore_unknown=True)
    assert (row, dropped) == ({"a": 1}, 1)


# ---------------------------------------------------------------------------- nested types
ADDRESS = StructType(
    NestedField(field_id=10, name="zip", field_type=StringType(), required=True),
    NestedField(field_id=11, name="floor", field_type=IntegerType(), required=False),
)


def test_a_struct_maps_recursively() -> None:
    assert one(ADDRESS, {"zip": "1000", "floor": 3}) == {"zip": "1000", "floor": 3}
    assert one(ADDRESS, {"zip": "1000"}) == {"zip": "1000", "floor": None}


def test_a_list_maps_each_element() -> None:
    kind = ListType(element_id=20, element_type=DecimalType(8, 2), element_required=False)
    assert one(kind, ["1.10", None, 3]) == [Decimal("1.10"), None, Decimal("3.00")]


def test_a_map_takes_pairs_for_any_key_and_an_object_for_text_keys() -> None:
    by_text = MapType(key_id=30, key_type=StringType(), value_id=31, value_type=IntegerType(), value_required=False)
    assert one(by_text, {"a": 1, "b": 2}) == [("a", 1), ("b", 2)]
    by_int = MapType(key_id=32, key_type=IntegerType(), value_id=33, value_type=StringType(), value_required=False)
    assert one(by_int, [{"key": 1, "value": "x"}, {"key": 2, "value": None}]) == [(1, "x"), (2, None)]


def test_a_map_with_the_same_key_twice_is_refused() -> None:
    by_int = MapType(key_id=32, key_type=IntegerType(), value_id=33, value_type=StringType(), value_required=False)
    refused(by_int, [{"key": 1, "value": "a"}, {"key": 1, "value": "b"}], "TYPE")


def test_a_failure_inside_a_struct_names_the_path_of_the_column() -> None:
    kind = StructType(NestedField(field_id=40, name="inner", field_type=ADDRESS, required=False))
    with pytest.raises(Failure) as caught:
        map_record(schema(column("customer", kind)), {"customer": {"inner": {"zip": "1000", "floor": 1.5}}})
    assert (caught.value.column, caught.value.reason) == ("customer.inner.floor", "TYPE")


def test_a_nested_field_missing_from_the_table_fails_even_when_unknown_fields_are_ignored() -> None:
    with pytest.raises(Failure) as caught:
        map_record(schema(column("customer", ADDRESS)), {"customer": {"zip": "1", "ghost": 1}}, ignore_unknown=True)
    assert (caught.value.column, caught.value.reason) == ("customer.ghost", "UNKNOWN_FIELD")


def test_a_missing_required_nested_field_is_refused() -> None:
    with pytest.raises(Failure) as caught:
        map_record(schema(column("customer", ADDRESS)), {"customer": {"floor": 1}})
    assert (caught.value.column, caught.value.reason) == ("customer.zip", "MISSING_REQUIRED")


# ---------------------------------------------------------------------------- the mapping of the step
def test_a_mapping_reads_the_pointer_and_writes_the_column() -> None:
    table = schema(column("amount", DecimalType(10, 2), field_id=1), column("customer_name", StringType(), field_id=2))
    row, dropped = map_record(
        table,
        {"amount": "9.99", "customer": {"name": "Ana"}, "internal": "never written"},
        mapping=[("/amount", "amount"), ("/customer/name", "customer_name")],
    )
    assert row == {"amount": Decimal("9.99"), "customer_name": "Ana"}
    assert dropped == 0  # a field the mapping does not name is not written, and is not an unknown field


def test_a_pointer_decodes_its_escapes_and_a_missing_one_is_a_missing_value() -> None:
    table = schema(column("a", StringType(), field_id=1), column("b", StringType(), field_id=2))
    row, _ = map_record(table, {"x/y": "1", "t~": "2"}, mapping=[("/x~1y", "a"), ("/t~0", "b")])
    assert row == {"a": "1", "b": "2"}
    row, _ = map_record(table, {}, mapping=[("/gone", "a")])
    assert row == {"a": None, "b": None}


def test_a_mapping_to_a_column_the_table_does_not_have_is_refused() -> None:
    with pytest.raises(Failure) as caught:
        map_record(schema(column("a", StringType())), {"x": "1"}, mapping=[("/x", "ghost")])
    assert (caught.value.column, caught.value.reason) == ("ghost", "UNKNOWN_FIELD")


def test_a_required_column_that_the_mapping_leaves_out_is_refused() -> None:
    table = schema(column("a", StringType(), required=True), column("b", StringType(), field_id=2))
    with pytest.raises(Failure) as caught:
        map_record(table, {"a": "1", "b": "1"}, mapping=[("/b", "b")])
    assert (caught.value.column, caught.value.reason) == ("a", "MISSING_REQUIRED")


def test_a_map_object_is_for_text_keys_only() -> None:
    by_int = MapType(key_id=32, key_type=IntegerType(), value_id=33, value_type=StringType(), value_required=False)
    refused(by_int, {"1": "x"}, "TYPE")


def test_a_pointer_decodes_tilde_one_after_tilde_zero_not_before() -> None:
    # `~01` is the two characters `~1`, not a slash
    table = schema(column("a", StringType()))
    row, _ = map_record(table, {"~1": "v", "/": "wrong"}, mapping=[("/~01", "a")])
    assert row == {"a": "v"}


@pytest.mark.parametrize("segment", ["0", "1", "3"])
def test_an_array_index_in_a_pointer_selects_the_element(segment: str) -> None:
    table = schema(column("a", StringType()))
    row, _ = map_record(table, {"items": ["w", "x", "y", "z"]}, mapping=[(f"/items/{segment}", "a")])
    assert row == {"a": "wxyz"[int(segment)]}


# RFC 6901: an index is "0" or digits that do not start with 0. Python's `str.isdigit` is true for "²" and "٣" (and
# `int("²")` raises), and `int("01")` is 1, so the check must be ASCII and must refuse a leading zero. A segment that
# is not an index is a missing value, as for any other pointer that leads nowhere.
@pytest.mark.parametrize(
    "segment", ["²", "٣", "①", "00", "01", "+1", "-1", " 1", "1 ", "1.0", "4", "99999999999999999999"]
)
def test_a_segment_that_is_not_an_array_index_leads_nowhere(segment: str) -> None:
    table = schema(column("a", StringType()))
    row, _ = map_record(table, {"items": ["w", "x", "y", "z"]}, mapping=[(f"/items/{segment}", "a")])
    assert row == {"a": None}


def test_a_null_element_of_a_required_list_element_is_refused() -> None:
    kind = ListType(element_id=20, element_type=IntegerType(), element_required=True)
    with pytest.raises(Failure) as caught:
        map_record(schema(column("x", kind)), {"x": [1, None]})
    assert (caught.value.column, caught.value.reason) == ("x[]", "NULL_IN_REQUIRED")


# ---------------------------------------------------------------------------- numbers written to cost something
@contextmanager
def within(seconds: int) -> Iterator[None]:
    """Fails the test when the block takes longer: a number that makes the mapping work for a minute is a defect."""

    def too_slow(signum: int, frame: object) -> None:
        del signum, frame
        raise AssertionError(f"the mapping took more than {seconds} s")

    previous = signal.signal(signal.SIGALRM, too_slow)
    signal.alarm(seconds)
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)


@pytest.mark.parametrize("kind", [IntegerType(), LongType()])
@pytest.mark.parametrize("exponent", [20, 1_000, 1_000_000, 999_999_999])
@pytest.mark.parametrize("sign", ["", "-"])
def test_a_number_with_a_huge_exponent_is_an_overflow_and_costs_nothing(
    kind: IcebergType, exponent: int, sign: str
) -> None:
    # `1e1000000` read with parse_float=Decimal is a Decimal of one digit; turning it into an integer first would build
    # a number of a million digits (twenty seconds, and quadratic) before comparing it with the width of the column
    with within(2):
        refused(kind, Decimal(f"{sign}1E+{exponent}"), "OVERFLOW")


def test_zero_is_zero_whatever_its_exponent() -> None:
    with within(2):
        assert one(LongType(), Decimal("0E+1000000000")) == 0
        assert one(IntegerType(), Decimal("-0E+5")) == 0


@pytest.mark.parametrize(
    ("kind", "inside", "outside"),
    [
        (LongType(), "9.223372036854775807E+18", "9.223372036854775808E+18"),
        (LongType(), "-9.223372036854775808E+18", "-9.223372036854775809E+18"),
        (IntegerType(), "2.147483647E+9", "2.147483648E+9"),
        (IntegerType(), "-2.147483648E+9", "-2.147483649E+9"),
    ],
)
def test_the_edges_of_the_width_hold_for_numbers_written_with_an_exponent(
    kind: IcebergType, inside: str, outside: str
) -> None:
    assert one(kind, Decimal(inside)) == int(Decimal(inside))
    refused(kind, Decimal(outside), "OVERFLOW")


def test_a_tiny_fraction_is_a_type_error_and_costs_nothing() -> None:
    with within(2):
        refused(LongType(), Decimal("1E-1000000"), "TYPE")


@pytest.mark.parametrize("kind", [IntegerType(), LongType()])
def test_a_string_of_more_digits_than_python_converts_is_an_overflow_and_not_a_crash(kind: IcebergType) -> None:
    refused(kind, "9" * 5_000, "OVERFLOW")
    refused(kind, "-" + "9" * 5_000, "OVERFLOW")


def test_leading_zeros_do_not_count_as_digits_of_the_value() -> None:
    assert one(LongType(), "0" * 5_000 + "1") == 1
    assert one(LongType(), "-" + "0" * 5_000 + "7") == -7
    assert one(IntegerType(), "0" * 5_000) == 0


@pytest.mark.parametrize(
    ("kind", "value"),
    [
        (DoubleType(), Decimal("1E-400")),
        (DoubleType(), Decimal("-1E-999999999")),
        (FloatType(), Decimal("1E-46")),
        (FloatType(), Decimal("-1E-50")),
        (FloatType(), 1e-46),  # a float already, and not zero in a double
    ],
)
def test_a_value_so_small_that_it_would_become_zero_is_refused_and_not_stored_as_zero(
    kind: IcebergType, value: object
) -> None:
    refused(kind, value, "OVERFLOW")


@pytest.mark.parametrize(
    ("kind", "value", "stored"),
    [
        (DoubleType(), Decimal("0E-1000"), 0.0),
        (FloatType(), Decimal("0E-1000"), 0.0),
        (FloatType(), 0, 0.0),
        (DoubleType(), 0.0, 0.0),
        (DoubleType(), Decimal("5E-324"), 5e-324),
        (FloatType(), Decimal("2E-45"), 2e-45),
        (FloatType(), Decimal("1.17549435E-38"), 1.17549435e-38),
        (DoubleType(), Decimal("2.2250738585072014E-308"), 2.2250738585072014e-308),
    ],
)
def test_zero_and_the_smallest_values_a_column_can_hold_are_stored(
    kind: IcebergType, value: object, stored: float
) -> None:
    assert one(kind, value) == pytest.approx(stored, rel=1e-6, abs=0)


@pytest.mark.parametrize(
    "key_type",
    [
        StructType(NestedField(50, "a", IntegerType(), required=False)),
        ListType(element_id=51, element_type=IntegerType(), element_required=False),
        MapType(key_id=52, key_type=StringType(), value_id=53, value_type=IntegerType(), value_required=False),
    ],
)
def test_a_map_key_that_is_not_a_primitive_is_refused_and_does_not_crash(key_type: IcebergType) -> None:
    # a struct, a list or a map as a key is legal in the format and unhashable in Python: the same key twice could not
    # even be looked for. Keys are primitive; anything else is a type mismatch.
    kind = MapType(key_id=40, key_type=key_type, value_id=41, value_type=IntegerType(), value_required=False)
    refused(kind, [{"key": {"a": 1}, "value": 1}], "TYPE")
    refused(kind, [], "TYPE")
