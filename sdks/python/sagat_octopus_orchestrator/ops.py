"""The operator factory of a collection (design 042, FR-042-028).

Each function returns one stage of the ``operators`` of a ``forEach``: an object with exactly one key, the
operator's name. Pass a sequence of them as ``operators=`` to ``for_each`` or ``map``::

    from sagat_octopus_orchestrator import ops
    flow.for_each("lines", "input.lines", "line", "bodyResult", max_items=100, max_concurrency=4,
                  operators=[ops.filter("element.total > 0"), ops.distinct("element.sku"), ops.window(25)])

The factory only assembles a stage; the server validates the chain (its length, the ranges of the counts and the
variables an expression reads). Stage expressions read ``element``, ``position``, ``input``, ``steps``, ``run``
and ``state``.
"""

from __future__ import annotations

from collections.abc import Mapping
from copy import deepcopy
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import Any

__all__ = ["Operator", "distinct", "filter", "flat_map", "map", "skip", "take", "window"]


@dataclass(frozen=True)
class Operator:
    """One stage of a collection's operator chain, as ``workflow.schema.json`` names it."""

    name: str
    parameters: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "parameters", MappingProxyType(dict(self.parameters)))

    def to_json(self) -> dict[str, Any]:
        """The stage as the IR writes it, ``{name: parameters}``; a fresh copy on every call."""
        return {self.name: deepcopy(dict(self.parameters))}


def filter(where: str) -> Operator:
    """Keeps the elements for which ``where`` is true."""
    return Operator("filter", {"where": where})


def map(select: str) -> Operator:
    """Replaces each element by the value of ``select``."""
    return Operator("map", {"select": select})


def flat_map(select: str) -> Operator:
    """``select`` yields a list for each element; the lists are concatenated in order, one level deep."""
    return Operator("flatMap", {"select": select})


def skip(count: int) -> Operator:
    """Drops the first ``count`` elements."""
    return Operator("skip", {"count": count})


def take(count: int) -> Operator:
    """Keeps the first ``count`` elements."""
    return Operator("take", {"count": count})


def distinct(by: str | None = None) -> Operator:
    """Keeps the first element of each value of ``by``, or of the element itself when ``by`` is absent."""
    return Operator("distinct", {} if by is None else {"by": by})


def window(count: int) -> Operator:
    """Replaces the list by consecutive lists of ``count`` elements, the last one possibly shorter."""
    return Operator("window", {"count": count})
