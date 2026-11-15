"""``reapplyOnReset`` on an Update declaration (feature 018, T018-078): an optional boolean, written into the
document only when true, so a document that does not opt in is the one made before the field existed."""

from __future__ import annotations

from dataclasses import replace
from typing import Any

import pytest

from sagat_octopus_orchestrator import SagatError, StateDefinition, UpdateDefinition, workflow

PLAIN = UpdateDefinition(
    args_schema={"type": "integer"},
    state="state",
    result="newState.balance",
    result_schema={"type": "integer"},
    disclosure="declared-business-result-v1",
)


def declared_update(update: UpdateDefinition) -> dict[str, Any]:
    document = (
        workflow("account-state")
        .state(
            StateDefinition(
                schema={
                    "type": "object",
                    "properties": {"balance": {"type": "integer"}},
                    "required": ["balance"],
                    "additionalProperties": False,
                },
                initial="input.account",
                updates={"balance.credit": update},
            )
        )
        .end("done")
        .build()
    )
    declared: dict[str, Any] = document["state"]["updates"]["balance.credit"]
    return declared


def test_an_update_that_opts_in_writes_the_field() -> None:
    assert declared_update(replace(PLAIN, reapply_on_reset=True))["reapplyOnReset"] is True


def test_the_default_is_not_written() -> None:
    assert "reapplyOnReset" not in declared_update(PLAIN)
    assert "reapplyOnReset" not in declared_update(replace(PLAIN, reapply_on_reset=False))


@pytest.mark.parametrize("wrong", ["yes", 1, None, {}, [True]])
def test_a_value_that_is_not_a_boolean_is_refused_before_the_server_sees_it(wrong: Any) -> None:
    with pytest.raises(SagatError):
        declared_update(replace(PLAIN, reapply_on_reset=wrong))
