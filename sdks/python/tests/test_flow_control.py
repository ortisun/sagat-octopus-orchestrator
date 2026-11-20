"""Feature 022 (FR-022-035, FR-022-050): ``concurrency`` and ``rate_limit`` at the workflow level and on activity and
script steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication."""

import json
from pathlib import Path
from typing import Any

from sagat_octopus_orchestrator import (
    ConcurrencyPolicy,
    RateLimitPolicy,
    WorkflowBuilder,
    canonical_json,
    workflow,
)

FIXTURES = Path(__file__).parents[3] / "tests/contract/flow-control-step-policies/fixtures"
DIGEST = "sha256:" + "0" * 64


def provider_sync() -> WorkflowBuilder:
    return (
        workflow("provider-sync")
        .concurrency(ConcurrencyPolicy(key="input.accountId", limit=1))
        .rate_limit(
            RateLimitPolicy(
                group="provider-starts", key="input.providerId", limit=100, per="minute", queue_timeout_ms=900000
            )
        )
        .step("fetch", "provider.fetch", rate_limit=RateLimitPolicy(key="input.providerId", limit=10, per="second"))
        .end("done")
    )


def account_serial() -> WorkflowBuilder:
    return (
        workflow("account-serial")
        .concurrency(
            ConcurrencyPolicy(group="orders-by-account", key="input.accountId", limit=1, queue_timeout_ms=600000)
        )
        .step("price", "catalog.price")
        .step("notify", "crm.notify", concurrency=ConcurrencyPolicy(key="steps.price.output.region", limit=2))
        .end("done")
    )


def test_the_builders_write_the_shared_fixtures_byte_for_byte() -> None:
    for builder, name in ((provider_sync(), "provider-sync"), (account_serial(), "account-serial")):
        fixture = json.loads((FIXTURES / f"{name}.workflow.json").read_text())
        assert builder.build() == fixture, name
        assert builder.to_json() == canonical_json(fixture), name


def test_no_policy_writes_no_field_so_earlier_documents_are_unchanged() -> None:
    document = workflow("plain-flow").step("a", "act.a").script("s", DIGEST).end("done").build()
    for key in ("concurrency", "rateLimit"):
        assert key not in document
        assert all(key not in node for node in document["nodes"])


def test_a_script_carries_both_policies_with_every_member() -> None:
    node: dict[str, Any] = (
        workflow("gated-script")
        .script(
            "s",
            DIGEST,
            concurrency=ConcurrencyPolicy(
                key="input.tenant", limit=10000, group="scripts.a-b", queue_timeout_ms=86400000
            ),
            rate_limit=RateLimitPolicy(
                key="input.tenant", limit=3, per="hour", group="scripts.a-b", queue_timeout_ms=1
            ),
        )
        .end("done")
        .build()["nodes"][0]
    )
    assert node["concurrency"] == {
        "key": "input.tenant",
        "limit": 10000,
        "group": "scripts.a-b",
        "queueTimeoutMs": 86400000,
    }
    assert node["rateLimit"] == {
        "key": "input.tenant",
        "limit": 3,
        "per": "hour",
        "group": "scripts.a-b",
        "queueTimeoutMs": 1,
    }


def test_the_last_workflow_level_call_wins_and_the_document_is_a_copy() -> None:
    builder = (
        workflow("replaced-flow")
        .concurrency(ConcurrencyPolicy(limit=9))
        .concurrency(ConcurrencyPolicy(limit=2))
        .rate_limit(RateLimitPolicy(limit=9, per="hour"))
        .rate_limit(RateLimitPolicy(limit=4, per="second"))
        .step("a", "act.a")
        .end("done")
    )
    first = builder.build()
    first["concurrency"]["limit"] = 7
    document = builder.build()
    assert document["concurrency"] == {"limit": 2}
    assert document["rateLimit"] == {"limit": 4, "per": "second"}


def test_the_builders_only_assemble_the_bounds_are_the_servers() -> None:
    document = (
        workflow("unchecked-flow")
        .concurrency(ConcurrencyPolicy(limit=0, group="Not A Group"))
        .rate_limit(RateLimitPolicy(limit=101, per="second", queue_timeout_ms=86400001))
        .step("a", "act.a", concurrency=ConcurrencyPolicy(limit=10001, key=""))
        .end("done")
        .build()
    )
    assert document["concurrency"] == {"limit": 0, "group": "Not A Group"}
    assert document["rateLimit"] == {"limit": 101, "per": "second", "queueTimeoutMs": 86400001}
    assert document["nodes"][0]["concurrency"] == {"limit": 10001, "key": ""}
