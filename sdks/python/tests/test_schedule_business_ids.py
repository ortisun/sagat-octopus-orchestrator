"""The business-id prefix of a schedule's action (feature 018, FR-018-016): what a write sends, what a read keeps, that
the customer's text is never altered by the SDK nor printed by it, and what the conformance runner prints of it."""

from __future__ import annotations

import pickle
from collections.abc import Callable
from typing import Any

import pytest

from conformance.runner import description_json, schedule_json, write_of
from sagat_octopus_orchestrator import (
    SagatClient,
    ScheduleAction,
    ScheduleActionSummary,
    ScheduleWrite,
)
from sagat_octopus_orchestrator.operations import to_schedule, to_schedule_description

from .fakes import FakeHttpServer, reply
from .test_client import refusal
from .test_operations import NS, RUN, SCHEDULE, WF, action_body, only, refused, schedule_body, simple_write

MakeClient = Callable[..., SagatClient]

# Text a log scanner can find: any appearance of it in a printed value is a leak.
SECRET_PREFIX = "Cust0mer/Prefix~42"


def write_with(**action: Any) -> ScheduleWrite:
    return simple_write(action=ScheduleAction(workflow_id=WF, input={}, **action))


def served_action(**fields: Any) -> dict[str, Any]:
    return {"workflowId": WF, "versionPolicy": "ACTIVE", "inputDigest": "a" * 64, **fields}


# ------------------------------------------------------------------------------------------------- the write


async def test_a_create_sends_the_prefix_and_both_policies_in_the_action_under_their_wire_names(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(201, schedule_body()))
    await make_client().schedules.create(
        NS,
        write_with(
            business_id_prefix="order",
            business_id_conflict_policy="USE_EXISTING",
            business_id_reuse_policy="REJECT_DUPLICATE",
        ),
        idempotency_key="sched-key-1",
    )
    assert only(http).body["action"] == {
        "workflowId": WF,
        "input": {},
        "businessIdPrefix": "order",
        "businessIdConflictPolicy": "USE_EXISTING",
        "businessIdReusePolicy": "REJECT_DUPLICATE",
    }


async def test_a_prefix_alone_sends_only_the_prefix_and_the_server_applies_the_defaults(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(201, schedule_body()))
    await make_client().schedules.create(NS, write_with(business_id_prefix="order"))
    assert only(http).body["action"] == {"workflowId": WF, "input": {}, "businessIdPrefix": "order"}


@pytest.mark.parametrize("call", ["create", "update"])
async def test_an_action_that_sets_none_of_the_three_sends_none_of_the_three_keys_not_null_and_not_empty(
    http: FakeHttpServer, make_client: MakeClient, call: str
) -> None:
    http.serve(lambda _seen: reply(200, schedule_body()))
    schedules = make_client().schedules
    if call == "create":
        await schedules.create(NS, simple_write())
    else:
        await schedules.update(NS, SCHEDULE, simple_write(), 2)
    action = only(http).body["action"]
    assert action == {"workflowId": WF, "input": {}}
    assert not {"businessIdPrefix", "businessIdConflictPolicy", "businessIdReusePolicy"} & action.keys()


async def test_an_update_takes_the_same_action_document_and_a_schedule_updated_without_a_prefix_sends_none(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, schedule_body(revision=3)))
    client = make_client()
    await client.schedules.update(
        NS,
        SCHEDULE,
        write_with(business_id_prefix="order", business_id_reuse_policy="ALLOW_DUPLICATE_FAILED_ONLY"),
        2,
        idempotency_key="upd-key-0001",
    )
    request = only(http)
    assert request.method == "PUT"
    assert request.body["action"] == {
        "workflowId": WF,
        "input": {},
        "businessIdPrefix": "order",
        "businessIdReusePolicy": "ALLOW_DUPLICATE_FAILED_ONLY",
    }
    assert request.body["expectedRevision"] == 2
    http.seen.clear()
    await client.schedules.update(NS, SCHEDULE, simple_write(), 3, idempotency_key="upd-key-0002")
    assert only(http).body["action"] == {"workflowId": WF, "input": {}}


@pytest.mark.parametrize(
    "prefix",
    [
        "",
        " order ",
        "ORDER-Mixed_case",
        "a" * 161,
        "with space",
        "naïve-ü",
        "tab\tinside",
        "‮RTL",
        "0",
    ],
)
async def test_the_prefix_is_customer_text_sent_as_given_with_no_trimming_no_case_folding_and_no_local_check(
    http: FakeHttpServer, make_client: MakeClient, prefix: str
) -> None:
    http.serve(lambda _seen: reply(201, schedule_body()))
    await make_client().schedules.create(NS, write_with(business_id_prefix=prefix))
    action = only(http).body["action"]
    assert action["businessIdPrefix"] == prefix, "the same text, byte for byte"
    assert "businessIdConflictPolicy" not in action


async def test_a_prefix_the_server_refuses_is_the_servers_validation_failed_with_its_reason_and_the_sdk_never_says_the_prefix(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            422,
            refusal(
                "VALIDATION_FAILED",
                "the schedule is invalid",
                False,
                details={"reason": "SCHEDULE_SPEC_INVALID", "field": "/action/businessIdPrefix"},
            ),
        )
    )
    error = await refused(make_client().schedules.create(NS, write_with(business_id_prefix="")))
    assert (error.code, error.reason, error.status) == ("VALIDATION_FAILED", "SCHEDULE_SPEC_INVALID", 422)
    assert len(http.seen) == 1, "a validation failure is never retried"
    assert only(http).body["action"]["businessIdPrefix"] == "", "the empty string goes out as the empty string"

    # Nothing the SDK writes about a refusal repeats the customer's text, whatever the server answered.
    http.seen.clear()
    http.serve(lambda _seen: reply(500, "<html>an intermediary page</html>"))
    failed = await refused(make_client().schedules.create(NS, write_with(business_id_prefix=SECRET_PREFIX)))
    for text in (str(failed), repr(failed), failed.message, repr(failed.to_dict())):
        assert SECRET_PREFIX not in text


# -------------------------------------------------------------------------------------------------- the read


async def test_a_schedule_whose_action_has_a_prefix_reads_it_back_with_both_policies_as_the_strings_the_server_sent(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            201,
            schedule_body(
                action=served_action(
                    businessIdPrefix=" order ",
                    businessIdConflictPolicy="USE_EXISTING",
                    businessIdReusePolicy="ALLOW_DUPLICATE_FAILED_ONLY",
                )
            ),
        )
    )
    created = await make_client().schedules.create(NS, write_with(business_id_prefix=" order "))
    assert created.action.business_id_prefix == " order ", "kept as sent: no trimming"
    assert created.action.business_id_conflict_policy == "USE_EXISTING"
    assert created.action.business_id_reuse_policy == "ALLOW_DUPLICATE_FAILED_ONLY"
    assert created.raw["action"]["businessIdPrefix"] == " order "


async def test_a_policy_this_sdk_does_not_know_is_kept_as_the_raw_string_and_is_never_an_error(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            schedule_body(
                action=served_action(
                    businessIdPrefix="order",
                    businessIdConflictPolicy="SOMETHING_NEWER",
                    businessIdReusePolicy="SOMETHING_ELSE",
                )
            ),
        )
    )
    got = await make_client().schedules.get(NS, SCHEDULE)
    assert got.action.business_id_prefix == "order"
    assert got.action.business_id_conflict_policy == "SOMETHING_NEWER"
    assert got.action.business_id_reuse_policy == "SOMETHING_ELSE"


async def test_an_action_without_a_prefix_reads_none_of_the_three_whatever_the_call(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, schedule_body()))
    client = make_client()
    for schedule in (
        await client.schedules.create(NS, simple_write()),
        await client.schedules.update(NS, SCHEDULE, simple_write(), 2),
        await client.schedules.get(NS, SCHEDULE),
    ):
        action = schedule.action
        assert (action.business_id_prefix, action.business_id_conflict_policy, action.business_id_reuse_policy) == (
            None,
            None,
            None,
        )
    # A value that is not text is no value at all, the same as every other optional string the SDK reads.
    nulled = to_schedule(schedule_body(action=served_action(businessIdPrefix=None, businessIdConflictPolicy=None)))
    assert nulled.action.business_id_prefix is None
    assert nulled.action.business_id_conflict_policy is None


def test_the_listing_and_the_description_read_the_prefix_of_every_schedule_they_carry() -> None:
    page_item = to_schedule(schedule_body(action=served_action(businessIdPrefix="a", businessIdReusePolicy="X")))
    assert (page_item.action.business_id_prefix, page_item.action.business_id_reuse_policy) == ("a", "X")
    description = to_schedule_description(
        {**schedule_body(action=served_action(businessIdPrefix="b", businessIdConflictPolicy="FAIL"))}
    )
    assert (description.action.business_id_prefix, description.action.business_id_conflict_policy) == ("b", "FAIL")


async def test_the_new_outcomes_and_reasons_of_a_business_id_are_kept_as_the_strings_the_server_sent(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    records = [
        action_body(outcome="BUSINESS_ID_CONFLICT", reason="BUSINESS_ID_IN_USE", runId=None),
        action_body(outcome="BUSINESS_ID_CONFLICT", reason="BUSINESS_ID_REUSE_REJECTED", runId=None),
        action_body(outcome="ALREADY_ACTED", reason="BUSINESS_ID_IN_USE", runId=RUN),
    ]
    http.serve(lambda _seen: reply(200, {"items": records, "nextCursor": None}))
    listed = await make_client().schedules.list_actions(NS, SCHEDULE, outcome="BUSINESS_ID_CONFLICT")
    assert only(http).url.endswith("outcome=BUSINESS_ID_CONFLICT")
    assert [(item.outcome, item.reason, item.run_id) for item in listed.items] == [
        ("BUSINESS_ID_CONFLICT", "BUSINESS_ID_IN_USE", None),
        ("BUSINESS_ID_CONFLICT", "BUSINESS_ID_REUSE_REJECTED", None),
        ("ALREADY_ACTED", "BUSINESS_ID_IN_USE", RUN),
    ]
    http.serve(lambda _seen: reply(202, records[2]))
    triggered = await make_client().schedules.trigger(NS, SCHEDULE)
    assert (triggered.outcome, triggered.reason, triggered.run_id) == ("ALREADY_ACTED", "BUSINESS_ID_IN_USE", RUN)
    http.serve(lambda _seen: reply(200, {**schedule_body(), "recentActions": records}))
    described = await make_client().schedules.get(NS, SCHEDULE)
    assert [item.outcome for item in described.recent_actions] == [
        "BUSINESS_ID_CONFLICT",
        "BUSINESS_ID_CONFLICT",
        "ALREADY_ACTED",
    ]


# ---------------------------------------------------------------------------------------- never printed


def test_the_prefix_does_not_appear_in_the_printed_form_of_a_write_or_of_what_is_read_back() -> None:
    action = ScheduleAction(
        workflow_id=WF,
        input={},
        business_id_prefix=SECRET_PREFIX,
        business_id_conflict_policy="USE_EXISTING",
        business_id_reuse_policy="REJECT_DUPLICATE",
    )
    write = simple_write(action=action)
    served = served_action(
        businessIdPrefix=SECRET_PREFIX,
        businessIdConflictPolicy="USE_EXISTING",
        businessIdReusePolicy="REJECT_DUPLICATE",
    )
    schedule = to_schedule(schedule_body(action=served))
    description = to_schedule_description({**schedule_body(action=served), "recentActions": [action_body()]})
    for value in (action, write, schedule.action, schedule, description.action, description):
        for text in (repr(value), str(value), f"{value}", f"{value!r:>400}"):
            assert SECRET_PREFIX not in text, type(value).__name__
    # The policies are not customer text: they stay readable in the printed form.
    assert "USE_EXISTING" in repr(action) and "USE_EXISTING" in repr(schedule.action)
    # Hidden from the printed form, not from the value: the caller still reads and compares it.
    assert action.business_id_prefix == SECRET_PREFIX and schedule.action.business_id_prefix == SECRET_PREFIX
    assert action != ScheduleAction(workflow_id=WF, input={}, business_id_prefix="other")


def test_the_new_fields_are_optional_and_keyword_compatible_so_no_existing_caller_breaks() -> None:
    assert ScheduleAction(WF, {}).business_id_prefix is None
    summary = ScheduleActionSummary(WF, None, None, None, None, None)
    assert (summary.business_id_prefix, summary.business_id_conflict_policy, summary.business_id_reuse_policy) == (
        None,
        None,
        None,
    )
    assert pickle.loads(pickle.dumps(ScheduleAction(WF, {}, business_id_prefix="p"))).business_id_prefix == "p"


# ------------------------------------------------------------------------------------ the conformance runner


def scenario_write(**action: Any) -> dict[str, Any]:
    return {
        "name": "crud",
        "spec": {"cron": ["0 9 * * *"]},
        "action": {"workflowId": WF, "input": {"n": 1}, **action},
    }


def test_the_runner_passes_the_three_fields_of_a_scenario_action_through_the_typed_write() -> None:
    write = write_of(
        scenario_write(
            businessIdPrefix=" order ",
            businessIdConflictPolicy="USE_EXISTING",
            businessIdReusePolicy="ALLOW_DUPLICATE_FAILED_ONLY",
        )
    )
    assert write.action.business_id_prefix == " order "
    assert write.action.business_id_conflict_policy == "USE_EXISTING"
    assert write.action.business_id_reuse_policy == "ALLOW_DUPLICATE_FAILED_ONLY"
    only_prefix = write_of(scenario_write(businessIdPrefix=""))
    assert only_prefix.action.business_id_prefix == "", "an empty prefix is passed on, the server refuses it"
    assert only_prefix.action.business_id_conflict_policy is None
    plain = write_of(scenario_write()).action
    assert (plain.business_id_prefix, plain.business_id_conflict_policy, plain.business_id_reuse_policy) == (
        None,
        None,
        None,
    )


async def test_a_scenario_write_with_the_three_fields_reaches_the_wire_as_it_was_written(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(201, schedule_body()))
    written = scenario_write(
        businessIdPrefix="Order/2", businessIdConflictPolicy="FAIL", businessIdReusePolicy="REJECT_DUPLICATE"
    )
    await make_client().schedules.create(NS, write_of(written))
    assert only(http).body["action"] == written["action"]


def test_the_runner_prints_the_three_keys_of_an_action_only_when_the_schedule_has_them() -> None:
    with_prefix = schedule_json(
        to_schedule(
            schedule_body(
                action=served_action(
                    businessIdPrefix="order",
                    businessIdConflictPolicy="FAIL",
                    businessIdReusePolicy="ALLOW_DUPLICATE",
                )
            )
        )
    )
    assert with_prefix["action"] == {
        "workflowId": WF,
        "versionPolicy": "ACTIVE",
        "versionId": None,
        "inputDigest": "a" * 64,
        "nominalTimeField": None,
        "runTimeoutMs": None,
        "businessIdPrefix": "order",
        "businessIdConflictPolicy": "FAIL",
        "businessIdReusePolicy": "ALLOW_DUPLICATE",
    }
    without = schedule_json(to_schedule(schedule_body()))
    assert without["action"] == {
        "workflowId": WF,
        "versionPolicy": "ACTIVE",
        "versionId": None,
        "inputDigest": "a" * 64,
        "nominalTimeField": None,
        "runTimeoutMs": None,
    }, "every existing golden stays byte-identical"
    odd = schedule_json(
        to_schedule(schedule_body(action=served_action(businessIdPrefix="p", businessIdReusePolicy="X")))
    )
    assert odd["action"]["businessIdPrefix"] == "p"
    assert odd["action"]["businessIdReusePolicy"] == "X", "a value this SDK does not know is printed as it came"
    assert "businessIdConflictPolicy" not in odd["action"], "an absent value is not printed, not even as null"


def test_the_runner_prints_the_same_action_for_a_described_schedule_and_for_a_recent_record_it_prints_what_it_did() -> (
    None
):
    served = {
        **schedule_body(action=served_action(businessIdPrefix="order", businessIdConflictPolicy="USE_EXISTING")),
        "recentActions": [action_body(outcome="ALREADY_ACTED", reason="BUSINESS_ID_IN_USE", runId=RUN)],
    }
    printed = description_json(to_schedule_description(served))
    assert printed["action"]["businessIdPrefix"] == "order"
    assert printed["action"]["businessIdConflictPolicy"] == "USE_EXISTING"
    assert "businessIdReusePolicy" not in printed["action"]
    assert (printed["recentActions"][0]["outcome"], printed["recentActions"][0]["reason"]) == (
        "ALREADY_ACTED",
        "BUSINESS_ID_IN_USE",
    )
    plain = description_json(to_schedule_description(schedule_body()))
    assert not {"businessIdPrefix", "businessIdConflictPolicy", "businessIdReusePolicy"} & plain["action"].keys()
