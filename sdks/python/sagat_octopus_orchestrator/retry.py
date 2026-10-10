"""Backoff with full jitter (``sdks/SPEC.md`` section 5)."""

from __future__ import annotations

import random
import re
from collections.abc import Callable
from dataclasses import dataclass

from .errors import SagatError


@dataclass(frozen=True)
class RetryPolicy:
    """How many times, and how patiently, a call that is safe to repeat is repeated. Delays are in seconds."""

    max_attempts: int = 4
    """Attempts in all, the first included."""
    initial_delay: float = 0.2
    max_delay: float = 5.0


DEFAULT_RETRY = RetryPolicy()

MAX_SERVER_WAIT = 60.0
"""Seconds. A wait that comes from the server (``Retry-After``, ``retry-after-ms``, the pause an empty poll asks
for) is never longer than this, whatever the server says."""

_MS = re.compile(r"[0-9]+")


def parse_ms(text: str | None) -> int | None:
    """Milliseconds from a header made of ASCII digits only; ``None`` for anything else (a sign, a space, a
    newline, digits of another script). More than 12 digits is clamped, not parsed (it is a cap's job)."""
    if text is None or not _MS.fullmatch(text):
        return None
    return int(text) if len(text) <= 12 else 10**12


def server_wait(milliseconds: int | None) -> float:
    """Seconds to wait for what a server asked, capped at ``MAX_SERVER_WAIT``."""
    return min(MAX_SERVER_WAIT, max(0, milliseconds or 0) / 1000)


def backoff_delay(policy: RetryPolicy, n: int, rng: Callable[[], float] = random.random) -> float:
    """``min(max_delay, initial_delay * 2**n)`` scaled by a random factor in [0.5, 1.0], in seconds.

    ``n`` is the attempt that just failed, counted from 0.
    """
    ceiling = min(policy.max_delay, policy.initial_delay * 2.0**n)
    return ceiling * (0.5 + rng() * 0.5)


def delay_after(policy: RetryPolicy, n: int, error: SagatError, rng: Callable[[], float] = random.random) -> float:
    """The wait before the next attempt: the backoff, or what the server asked for if that is longer."""
    return max(backoff_delay(policy, n, rng), server_wait(error.retry_after_ms))
