"""Backoff with full jitter (``sdks/SPEC.md`` section 5)."""

from __future__ import annotations

import random
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


def backoff_delay(policy: RetryPolicy, n: int, rng: Callable[[], float] = random.random) -> float:
    """``min(max_delay, initial_delay * 2**n)`` scaled by a random factor in [0.5, 1.0], in seconds.

    ``n`` is the attempt that just failed, counted from 0.
    """
    ceiling = min(policy.max_delay, policy.initial_delay * 2.0**n)
    return ceiling * (0.5 + rng() * 0.5)


def delay_after(policy: RetryPolicy, n: int, error: SagatError, rng: Callable[[], float] = random.random) -> float:
    """The wait before the next attempt: the backoff, or what the server asked for if that is longer."""
    asked = (error.retry_after_ms or 0) / 1000
    return max(backoff_delay(policy, n, rng), asked)
