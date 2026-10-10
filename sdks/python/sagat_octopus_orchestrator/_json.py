"""JSON on the way out and in, and instants (``sdks/SPEC.md`` sections 7 and 8).

Python's ``json`` keeps what the SDK promises: an ``int`` prints as an integer (``600000`` never becomes
``600000.0``), a ``str`` stays a string (``"19.99"`` is never turned into a number) and a ``float`` keeps its
shortest round-trip digits (``0.1`` stays ``0.1``). What it does not do is refuse what is not JSON, so the
checks are here.
"""

from __future__ import annotations

import json
import re
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any

from .errors import SagatError

_INT64_MIN = -(2**63)
_INT64_MAX = 2**63 - 1
_MAX_DEPTH = 200
_MAX_SAFE = 2**53 - 1


def utf16_key(text: str) -> bytes:
    """The sort key of RFC 8785: strings compare by UTF-16 code units, not by code points (a character above
    U+FFFF sorts before U+E000..U+FFFF, as in JavaScript, Java and C#)."""
    return text.encode("utf-16-be", "surrogatepass")


def _refuse(message: str) -> SagatError:
    return SagatError("VALIDATION_FAILED", message)


def _check(value: Any, path: str, depth: int) -> None:
    if depth > _MAX_DEPTH:
        raise _refuse(f"{path} is nested too deeply, or refers to itself")
    if value is None or isinstance(value, bool):
        return
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError:
            raise _refuse(f"{path} is a string that is not valid Unicode") from None
    elif isinstance(value, int):
        if not _INT64_MIN <= value <= _INT64_MAX:
            raise _refuse(f"{path} is an integer beyond 64 bits: send it as a string")
    elif isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise _refuse(f"{path} is not a finite number")
    elif isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                raise _refuse(f"{path} has a key that is not a string ({type(key).__name__})")
            try:
                key.encode("utf-8")
            except UnicodeEncodeError:
                raise _refuse(f"{path} has a key that is not valid Unicode") from None
            _check(item, f"{path}.{key}", depth + 1)
    elif isinstance(value, (list, tuple)):
        for index, item in enumerate(value):
            _check(item, f"{path}[{index}]", depth + 1)
    else:
        raise _refuse(f"{path} is a {type(value).__name__}, which is not JSON")


def encode_json(value: Any) -> str:
    """The JSON text of a value to send.

    Refuses what cannot be sent as it is: ``NaN`` and the infinities, an integer beyond 64 bits (a JSON
    reader would round it), a key that is not a string, a string that is not valid Unicode and any type
    that JSON has no form for (``Decimal``, ``datetime``, ``bytes``...: the SDK never guesses a
    representation).
    """
    _check(value, "$", 0)
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def _reject_constant(name: str) -> Any:
    raise ValueError(f"{name} is not JSON")


def decode_json(text: str | bytes) -> Any:
    """Parses JSON; ``NaN`` and the infinities, which Python's parser lets through, are refused."""
    return json.loads(text, parse_constant=_reject_constant)


def _es_number(value: float) -> str:
    """A finite ``float`` as ECMAScript's ``Number::toString`` writes it (``1.0`` is ``1``, ``1e21`` is ``1e+21``)."""
    if value == 0:
        return "0"
    sign = "-" if value < 0 else ""
    digits_tuple = Decimal(repr(abs(value))).as_tuple()
    digits = "".join(str(d) for d in digits_tuple.digits).rstrip("0") or "0"
    exponent = int(digits_tuple.exponent) + (len(digits_tuple.digits) - len(digits))  # value = digits * 10**exponent
    k = len(digits)
    n = k + exponent  # the decimal point sits after n digits
    if k <= n <= 21:
        return sign + digits + "0" * (n - k)
    if 0 < n <= 21:
        return f"{sign}{digits[:n]}.{digits[n:]}"
    if -6 < n <= 0:
        return f"{sign}0.{'0' * -n}{digits}"
    e = n - 1
    mantissa = digits if k == 1 else f"{digits[0]}.{digits[1:]}"
    return f"{sign}{mantissa}e{'+' if e >= 0 else '-'}{abs(e)}"


def _canonical(value: Any, out: list[str]) -> None:
    if value is None:
        out.append("null")
    elif value is True:
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, str):
        out.append(json.dumps(value, ensure_ascii=False))
    elif isinstance(value, int):
        if not -_MAX_SAFE <= value <= _MAX_SAFE:
            # The text must be the same bytes in every SDK, and TypeScript's cannot hold this integer exactly.
            raise _refuse("an integer beyond 2^53 - 1 has no canonical text: send it as a string")
        out.append(str(int(value)))
    elif isinstance(value, float):
        out.append(_es_number(value))
    elif isinstance(value, dict):
        out.append("{")
        for index, key in enumerate(sorted(value, key=utf16_key)):
            if index:
                out.append(",")
            out.append(json.dumps(key, ensure_ascii=False))
            out.append(":")
            _canonical(value[key], out)
        out.append("}")
    else:
        out.append("[")
        for index, item in enumerate(value):
            if index:
                out.append(",")
            _canonical(item, out)
        out.append("]")


def canonical_json(value: Any) -> str:
    """Canonical JSON text: keys sorted at every depth, no insignificant whitespace, numbers written as
    ``JSON.stringify`` of the TypeScript SDK writes them, so that the five SDKs agree byte for byte."""
    _check(value, "$", 0)
    out: list[str] = []
    _canonical(value, out)
    return "".join(out)


# ------------------------------------------------------------------------------------------- instants

_INSTANT = re.compile(
    r"([0-9]{4})-([0-9]{2})-([0-9]{2})[Tt ]([0-9]{2}):([0-9]{2}):([0-9]{2})"
    r"(?:\.([0-9]+))?(?:([Zz])|([+-])([0-9]{2}):?([0-9]{2}))"
)


def format_instant(value: datetime) -> str:
    """An RFC 3339 instant in UTC with a ``Z`` and milliseconds (``2030-01-01T00:00:00.000Z``).

    A naive ``datetime`` is refused: it does not say which instant it is, and the SDK never guesses a
    local offset.
    """
    if value.tzinfo is None or value.utcoffset() is None:
        raise _refuse("an instant needs a time zone: use an aware datetime (datetime.now(timezone.utc))")
    utc = value.astimezone(timezone.utc)
    return f"{utc:%Y-%m-%dT%H:%M:%S}.{utc.microsecond // 1000:03d}Z"


def parse_instant(text: Any) -> datetime | None:
    """An RFC 3339 instant with any offset, as an aware ``datetime`` in UTC; ``None`` if it is not one."""
    if not isinstance(text, str):
        return None
    match = _INSTANT.fullmatch(text)
    if not match:
        return None
    year, month, day, hour, minute, second, fraction, zulu, sign, off_h, off_m = match.groups()
    try:
        micro = int((fraction or "0")[:6].ljust(6, "0"))
        moment = datetime(
            int(year), int(month), int(day), int(hour), int(minute), int(second), micro, tzinfo=timezone.utc
        )
        if not zulu:
            offset = timedelta(hours=int(off_h), minutes=int(off_m))
            moment = moment - offset if sign == "+" else moment + offset
    except (ValueError, OverflowError):
        return None
    return moment
