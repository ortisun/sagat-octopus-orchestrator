"""The pinned connection of a request, as what the catalog client is built with (T035-018, D035-105).

The runner sends the connection the way the platform stored it (``data-model.md`` §1: ``catalog``, ``storage`` and
``warehouseLocations``) next to the secrets it resolved, in ``credentials.catalog`` and ``credentials.storage``, which
are the secret documents of the data model (``{"clientId", "clientSecret"}``, ``{"token"}`` and ``{"accessKeyId",
"secretAccessKey", "sessionToken"?}``). The host turns that into three things and nothing else: the hygiene filter of
the attempt (the storage type, the warehouse locations, what the catalog may delegate), the properties of the REST
client, and the catalog.

It serves what it can test: a REST catalog with no authentication, OAuth2 or a bearer token, over S3 with static or
vended credentials. A Glue, Hive or JDBC catalog, SigV4, remote signing, GCS, ADLS and an assumed role are not served
yet, and a request for one is a break of the protocol, as is anything the host does not know and anything that does
not have the shape of the data model. The failure is ``LAKE_HOST_PROTOCOL`` with no field name and no value; only a
static credential that is missing is told apart, with the code the hygiene already has for it, since that one is a
fact about the secret and not about the line.

An address that would carry a secret in the clear is refused: ``https`` everywhere, and ``http`` only on the loopback,
which is where the tests of the platform and the forwarder of the sandbox put it. No address has user information, a
query or a fragment.
"""

from __future__ import annotations

import re
from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import urlsplit

from .config_hygiene import Declaration, Hygiene, HygieneError, StorageType, clean_path
from .failures import LAKE_HOST_PROTOCOL, Failure
from .rest_catalog import LakeRestCatalog

_LOOPBACK = frozenset({"127.0.0.1", "localhost", "::1"})
_REGION = re.compile(r"[a-z0-9-]{1,32}")
_PREFIX = re.compile(r"[A-Za-z0-9._~/-]{1,256}")
_MAX_URL = 2048
_MAX_SECRET = 16384
_MAX_TEXT = 1024
_DELEGATION = {"none": False, "vended-credentials": True}
_STORAGE_CREDENTIALS = {"static": False, "vended": True}


@dataclass(frozen=True)
class OpenConnection:
    """What a request's connection comes to. The properties hold secrets and are never part of a representation."""

    hygiene: Hygiene = field(repr=False)
    properties: Mapping[str, str] = field(repr=False)
    # The warehouse locations the connection declares, and whether the catalog vends the storage credentials (so the
    # storage is reached with none of its own): what a probe of the storage needs to know and nothing it could leak.
    locations: tuple[str, ...] = ()
    vended: bool = False

    def catalog(self, *, request_timeout: float) -> LakeRestCatalog:
        """The catalog, which believes nothing its server says and gives every request a time limit."""
        return LakeRestCatalog(
            "lake",
            hygiene=self.hygiene,
            request_timeout=request_timeout,
            **self.hygiene.properties(self.properties, {}),
        )


def _break() -> Failure:
    return Failure(LAKE_HOST_PROTOCOL)


def open_connection(connection: object, *, development: bool = False) -> OpenConnection:
    """The hygiene and the client properties of ``connection``, or the failure that stops the attempt.

    ``development`` is the runner's own flag (O035-010, option b): only then is a storage endpoint or an OAuth2 token
    endpoint read over plain ``http``, and then on the loopback alone, as the catalog's address is everywhere.
    """
    try:
        return _open(connection, development=development)
    except HygieneError as error:
        raise Failure(error.code) from None
    except ValueError:
        raise _break() from None


def _open(connection: object, *, development: bool) -> OpenConnection:
    document = _object(connection, {"catalog", "storage", "warehouseLocations", "credentials"})
    catalog = _object(document["catalog"], {"type", "rest"})
    storage = _object(document["storage"], {"type", "credentials", "s3"})
    if catalog["type"] != "rest" or storage["type"] != "s3":
        raise _break()
    rest = _object(
        catalog["rest"],
        {"uri", "warehouse", "accessDelegation", "prefix", "auth", "remoteSigning"},
        required={"uri", "warehouse", "accessDelegation"},
    )
    s3 = _object(
        storage["s3"], {"region", "pathStyle", "endpoint", "roleArn", "externalId"}, required={"region", "pathStyle"}
    )
    if "roleArn" in s3 or "externalId" in s3 or rest.get("remoteSigning", False) is not False:
        raise _break()

    delegated = _choice(rest["accessDelegation"], _DELEGATION)
    vended = _choice(storage["credentials"], _STORAGE_CREDENTIALS)
    if delegated != vended:
        # vended storage credentials need the catalog to be asked for them, and a catalog is never asked for what the
        # storage does not use
        raise _break()

    secrets = _object(document["credentials"], {"catalog", "storage"}, required=set())
    properties: dict[str, str] = {
        "uri": _url(rest["uri"], loopback_http=True),
        "warehouse": _text(rest["warehouse"], _MAX_TEXT),
    }
    if "prefix" in rest:
        properties["prefix"] = _prefix(rest["prefix"])
    properties.update(_auth(rest.get("auth"), secrets.get("catalog"), development=development))

    properties["s3.region"] = _region(s3["region"])
    properties["s3.force-virtual-addressing"] = "false" if _flag(s3["pathStyle"]) else "true"
    if "endpoint" in s3:
        properties["s3.endpoint"] = _url(s3["endpoint"], loopback_http=development)
    properties.update(_storage_secrets(secrets.get("storage"), vended=vended))

    locations = document["warehouseLocations"]
    if not isinstance(locations, list) or not all(isinstance(location, str) for location in locations):
        raise _break()
    hygiene = Hygiene(Declaration(StorageType.S3, tuple(locations), vended_credentials=vended, remote_signing=False))
    if "://" in properties["warehouse"]:
        hygiene.check_location(properties["warehouse"])
    if not vended:
        hygiene.require_explicit_credentials(properties)
    return OpenConnection(hygiene=hygiene, properties=properties, locations=tuple(locations), vended=vended)


def _auth(auth: object, secrets: object, *, development: bool) -> dict[str, str]:
    held = _secrets(secrets)
    if auth is None:
        if held:
            raise _break()
        return {}
    kind = _object(auth, {"type", "tokenUri", "scope"}, required={"type"})["type"]
    if kind == "bearer":
        _object(auth, {"type"})
        if held.keys() != {"token"}:
            raise _break()
        return {"token": held["token"]}
    if kind == "oauth2":
        described = _object(auth, {"type", "tokenUri", "scope"}, required={"type", "tokenUri"})
        if (
            held.keys() != {"clientId", "clientSecret"}
            or not 1 <= len(held["clientId"]) <= 256
            or ":" in held["clientId"]
        ):
            raise _break()
        properties = {
            "credential": f"{held['clientId']}:{held['clientSecret']}",
            "oauth2-server-uri": _url(described["tokenUri"], loopback_http=development),
        }
        if "scope" in described:
            properties["scope"] = _text(described["scope"], 256)
        return properties
    raise _break()


def _storage_secrets(secrets: object, *, vended: bool) -> dict[str, str]:
    held = _secrets(secrets)
    if vended:
        # the catalog hands the credentials over; a secret sent anyway would sit in the sandbox for nothing
        if held:
            raise _break()
        return {}
    if not held.keys() <= {"accessKeyId", "secretAccessKey", "sessionToken"}:
        raise _break()
    names = {
        "accessKeyId": "s3.access-key-id",
        "secretAccessKey": "s3.secret-access-key",
        "sessionToken": "s3.session-token",
    }
    return {names[name]: value for name, value in held.items()}


def _secrets(value: object) -> dict[str, str]:
    """A secret document: text to text, with no control character, and none is empty by being absent."""
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise _break()
    for name, secret in value.items():
        if not isinstance(name, str) or not isinstance(secret, str) or len(secret) > _MAX_SECRET:
            raise _break()
        if any(ord(character) < 0x20 or ord(character) == 0x7F for character in secret):
            raise _break()
    return dict(value)


def _object(value: object, keys: set[str], *, required: set[str] | None = None) -> dict[str, Any]:
    """An object with these keys at most and ``required`` (by default all of them) at least."""
    needed = keys if required is None else required
    if not isinstance(value, dict) or not needed <= value.keys() <= keys:
        raise _break()
    return value


def _choice(value: object, options: Mapping[str, bool]) -> bool:
    if not isinstance(value, str) or value not in options:
        raise _break()
    return options[value]


def _flag(value: object) -> bool:
    if not isinstance(value, bool):
        raise _break()
    return value


def _text(value: object, limit: int) -> str:
    if not isinstance(value, str) or not 1 <= len(value.encode("utf-8", "replace")) <= limit:
        raise _break()
    if any(ord(character) < 0x20 or ord(character) == 0x7F for character in value):
        raise _break()
    return value


def _region(value: object) -> str:
    if not isinstance(value, str) or not _REGION.fullmatch(value):
        raise _break()
    return value


def _prefix(value: object) -> str:
    if not isinstance(value, str) or not _PREFIX.fullmatch(value) or not clean_path(value):
        raise _break()
    return value


def _url(value: object, *, loopback_http: bool) -> str:
    """An address with no user information, query or fragment; ``https``, or ``http`` on the loopback if allowed."""
    if not isinstance(value, str) or not 1 <= len(value) <= _MAX_URL:
        raise _break()
    if any(ord(character) <= 0x20 or ord(character) == 0x7F for character in value):
        raise _break()
    parts = urlsplit(value)
    if not parts.hostname or "@" in parts.netloc or parts.query or parts.fragment:
        raise _break()
    parts.port  # noqa: B018 - a port that is not a number raises ValueError, which is a break
    if parts.scheme == "https":
        return value
    if parts.scheme == "http" and loopback_http and parts.hostname in _LOOPBACK:
        return value
    raise _break()
