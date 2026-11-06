"""A small checker of the part of JSON Schema that ``receipts.schema.json`` uses (feature 035: D035-062).

The runner rebuilds each receipt from typed fields and refuses one the schema does not describe, so the host must write
what the schema describes. The Rust side already tests its types against the schema; this is the same check for the
Python side, against the same file, without a dependency the image does not have. It knows the keywords of that schema
and **refuses to pass over one it does not know**, so a schema that grows a keyword cannot make the check vacuous.
"""

from __future__ import annotations

import re
from typing import Any

# Keywords that describe and do not constrain.
_ANNOTATIONS = {"$schema", "$id", "$defs", "title", "description", "default", "examples"}


def problems(value: Any, node: dict[str, Any], defs: dict[str, Any], path: str = "$") -> list[str]:
    """What is wrong with ``value`` against ``node``; empty when it conforms."""
    if "$ref" in node:
        name = node["$ref"].removeprefix("#/$defs/")
        return problems(value, defs[name], defs, path)
    found: list[str] = []
    for keyword, expected in node.items():
        if keyword in _ANNOTATIONS:
            continue
        if keyword == "type":
            found += _type(value, expected, path)
        elif keyword == "const":
            found += [] if value == expected else [f"{path}: not the constant"]
        elif keyword == "enum":
            found += [] if value in expected else [f"{path}: not one of the values"]
        elif keyword == "pattern":
            ok = isinstance(value, str) and re.search(expected, value) is not None
            found += [] if ok or not isinstance(value, str) else [f"{path}: does not match {expected}"]
        elif keyword == "maxLength":
            found += [f"{path}: too long"] if isinstance(value, str) and len(value) > expected else []
        elif keyword == "required":
            found += [f"{path}: missing {name}" for name in expected if isinstance(value, dict) and name not in value]
        elif keyword == "properties":
            for name, child in expected.items():
                if isinstance(value, dict) and name in value:
                    found += problems(value[name], child, defs, f"{path}.{name}")
        elif keyword == "allOf":
            for part in expected:
                found += problems(value, part, defs, path)
        elif keyword == "unevaluatedProperties":
            if expected is False and isinstance(value, dict):
                known = _evaluated(node, defs)
                found += [f"{path}: unevaluated property {name}" for name in value if name not in known]
            elif expected is not False:
                found.append(f"{path}: unsupported unevaluatedProperties")
        else:
            found.append(f"{path}: the checker does not know the keyword {keyword}")
    return found


def _type(value: Any, expected: str, path: str) -> list[str]:
    ok = {
        "object": isinstance(value, dict),
        "string": isinstance(value, str),
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
    }.get(expected)
    if ok is None:
        return [f"{path}: the checker does not know the type {expected}"]
    return [] if ok else [f"{path}: not {expected}"]


def _evaluated(node: dict[str, Any], defs: dict[str, Any]) -> set[str]:
    """The names of the properties the schema describes in ``node``, through its ``allOf`` and ``$ref``."""
    if "$ref" in node:
        return _evaluated(defs[node["$ref"].removeprefix("#/$defs/")], defs)
    names = set(node.get("properties", {}))
    for part in node.get("allOf", []):
        names |= _evaluated(part, defs)
    return names
