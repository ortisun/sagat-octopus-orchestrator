"""The catalog addresses on which the runner and the host must agree (feature 035: D035-114).

The runner builds what the sandbox may reach from the text of the catalog's address
(``crates/infra-lake/src/policy.rs``); the host connects to the address it reads from the same text
(``connection._url``). If the two parsers read a URL differently, the runner could allow one host while the client
connects to another. ``tests/contract/iceberg-writes/fixtures/urls/catalog-uris.json`` is the corpus both are held to:
the Rust test reads the same file. Where one side is stricter on purpose it is listed in that side's own test, not in
the corpus.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from sagat_lake.connection import _url
from sagat_lake.failures import Failure

CORPUS = Path(__file__).resolve().parents[3] / "tests/contract/iceberg-writes/fixtures/urls/catalog-uris.json"


def corpus() -> dict[str, list[str]]:
    document = json.loads(CORPUS.read_text(encoding="utf-8"))
    return {"accept": list(document["accept"]), "reject": list(document["reject"])}


def test_the_corpus_is_not_empty() -> None:
    cases = corpus()
    assert len(cases["accept"]) >= 8
    assert len(cases["reject"]) >= 15


@pytest.mark.parametrize("url", corpus()["accept"])
def test_the_host_accepts_the_addresses_the_runner_accepts(url: str) -> None:
    assert _url(url, loopback_http=True) == url


@pytest.mark.parametrize("url", corpus()["reject"])
def test_the_host_refuses_the_addresses_the_runner_refuses(url: str) -> None:
    # a refusal is a break of the protocol, or a ValueError from a port that is not a number: both end the attempt
    with pytest.raises((Failure, ValueError)):
        _url(url, loopback_http=True)
