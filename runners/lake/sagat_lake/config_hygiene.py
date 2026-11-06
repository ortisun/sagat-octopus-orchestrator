"""Configuration hygiene of the lake host (feature 035, T035-033, D035-050, FR-035-004).

A catalog is not trusted. What it sends in `GET /v1/config` and in `LoadTable` is reduced to a closed allowlist, the
FileIO implementation is pinned by the host, remote signing is refused unless the connection declares it, every
location a catalog response points to must lie under a warehouse location the connection declares, the ambient
credential chains are closed, and a missing credential fails closed. Each denial is counted by a closed kind. No
message, count or error carries a catalog value, a location, a host or a key: the tests of the contract check that.

The relay refuses hosts the connection did not declare. It sees host names, not buckets (G035-021), so the confinement
to the declared prefixes is done here, and by credentials scoped to those prefixes.
"""

from __future__ import annotations

import re
from collections import Counter
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from enum import StrEnum

LOCATION_NOT_ALLOWED = "ICEBERG_LOCATION_NOT_ALLOWED"
# errors.md reuses the codes of feature 017 for a credential that cannot be resolved.
CREDENTIAL_UNAVAILABLE = "DATABASE_CREDENTIAL_UNAVAILABLE"

MAX_LOCATIONS = 16
_MAX_VALUE = 1024
_PREFIX = re.compile(r"[A-Za-z0-9._~/-]{1,256}")
_ENCODED = re.compile(r"%(2e|2f|5c|00)", re.IGNORECASE)


class StorageType(StrEnum):
    S3 = "s3"
    GCS = "gcs"
    ADLS = "adls"


class Denial(StrEnum):
    """What was refused, as a closed kind: never a value."""

    KEY_NOT_ALLOWED = "key_not_allowed"
    LOCATION_NOT_ALLOWED = "location_not_allowed"
    CREDENTIAL_MISSING = "credential_missing"


class HygieneError(Exception):
    """A refusal that fails the attempt. Its message is its code, so no value can reach a log or a receipt."""

    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


_SCHEME = {StorageType.S3: "s3", StorageType.GCS: "gs", StorageType.ADLS: "abfss"}

# The FileIO implementation PyIceberg itself prefers first for the scheme of each storage type. The host sets it, so a
# catalog cannot name another class to import.
_FILE_IO = {
    StorageType.S3: "pyiceberg.io.pyarrow.PyArrowFileIO",
    StorageType.GCS: "pyiceberg.io.pyarrow.PyArrowFileIO",
    StorageType.ADLS: "pyiceberg.io.fsspec.FsspecFileIO",
}

# The credential keys a catalog may vend, by storage type. A region, an endpoint, a role, a proxy or a signer is never
# one of them: the endpoints derive from the region the connection declares. The other ADLS forms arrive with their
# tests in the phase of every storage type.
_VENDED = {
    StorageType.S3: frozenset({"s3.access-key-id", "s3.secret-access-key", "s3.session-token"}),
    StorageType.GCS: frozenset({"gcs.oauth2.token", "gcs.oauth2.token-expires-at"}),
    StorageType.ADLS: frozenset({"adls.sas-token"}),
}
_SIGNER = frozenset({"s3.signer", "s3.signer.uri", "s3.signer.endpoint"})

# What a table says about where its files are, besides its own location: a property that moves the data files.
_LOCATION_PROPERTIES = (
    "write.data.path",
    "write.metadata.path",
    "write.folder-storage.path",
    "write.object-storage.path",
)

# The only variables the host keeps. Everything else (credential chains, metadata endpoints, proxies, certificate
# bundles, the import path) is dropped, and the values below are set by the host.
_KEPT_ENVIRONMENT = (
    "PATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "PYTHONHASHSEED",
    "PYTHONUNBUFFERED",
    "PYTHONDONTWRITEBYTECODE",
)
_SET_ENVIRONMENT = {
    "AWS_EC2_METADATA_DISABLED": "true",
    "AWS_CONFIG_FILE": "/dev/null",
    "AWS_SHARED_CREDENTIALS_FILE": "/dev/null",
}


def scrubbed_environment(source: Mapping[str, str]) -> dict[str, str]:
    """The environment of the host: a short allowlist of `source`, plus the settings that close the ambient chains."""
    kept = {name: source[name] for name in _KEPT_ENVIRONMENT if name in source}
    return {**kept, **_SET_ENVIRONMENT}


def clean_path(path: str) -> bool:
    """True when `path` cannot climb out of its prefix: no control character, backslash, query, fragment, encoded dot or
    separator, and no `.` or `..` segment."""
    if any(ord(c) <= 0x20 or ord(c) == 0x7F or c in "\\?#" for c in path) or _ENCODED.search(path):
        return False
    return all(segment not in {".", ".."} for segment in path.split("/"))


def _valid_prefix(storage: StorageType, prefix: str) -> bool:
    scheme, separator, rest = prefix.partition("://")
    authority, _, path = rest.partition("/")
    if separator != "://" or scheme != _SCHEME[storage] or not authority or not clean_path(path):
        return False
    if any(ord(c) <= 0x20 or c in "\\?#" for c in authority):
        return False
    if storage is StorageType.ADLS:
        container, at, account = authority.partition("@")
        return bool(at) and bool(container) and bool(account) and ":" not in container and "@" not in account
    return "@" not in authority


@dataclass(frozen=True, slots=True)
class Declaration:
    """What the connection declares and the host trusts: the storage type, where writes may go, and what is allowed."""

    storage: StorageType
    warehouse_locations: tuple[str, ...]
    vended_credentials: bool = False
    remote_signing: bool = False

    def __post_init__(self) -> None:
        if not 1 <= len(self.warehouse_locations) <= MAX_LOCATIONS:
            raise ValueError(f"a connection declares 1 to {MAX_LOCATIONS} warehouse locations")
        if not all(_valid_prefix(self.storage, prefix) for prefix in self.warehouse_locations):
            raise ValueError(f"a warehouse location does not fit the {self.storage} storage type")
        normalized = tuple(prefix if prefix.endswith("/") else prefix + "/" for prefix in self.warehouse_locations)
        object.__setattr__(self, "warehouse_locations", normalized)


@dataclass(frozen=True, slots=True)
class AcceptedTable:
    """A `LoadTable` response that passed: the table location and the config that survived the filter."""

    location: str
    config: dict[str, str]


class Hygiene:
    """The filter of one attempt. It counts what it refuses, by kind, for the audit."""

    def __init__(self, declaration: Declaration) -> None:
        self._declaration = declaration
        self._denied: Counter[Denial] = Counter()

    @property
    def denied(self) -> Counter[Denial]:
        return Counter(self._denied)

    @property
    def denials(self) -> int:
        return sum(self._denied.values())

    # -- configuration ---------------------------------------------------------------------------------------------

    def catalog_config(self, defaults: Mapping[str, object], overrides: Mapping[str, object]) -> dict[str, str]:
        """`GET /v1/config`: `warehouse` and `prefix`, the overrides over the defaults; every other key is dropped."""
        accepted: dict[str, str] = {}
        for source in (defaults, overrides):
            for key, value in source.items():
                if not isinstance(value, str) or key not in ("warehouse", "prefix") or not self._plausible(key, value):
                    self._deny(Denial.KEY_NOT_ALLOWED)
                elif key == "warehouse" and "://" in value:
                    accepted[key] = self.check_location(value)
                else:
                    accepted[key] = value
        return accepted

    def table_config(self, config: Mapping[str, object]) -> dict[str, str]:
        """`LoadTable.config`: the vended credentials of the declared storage type, if declared, and the signer keys if
        remote signing is declared; every other key is dropped."""
        allowed: frozenset[str] = frozenset()
        if self._declaration.vended_credentials:
            allowed |= _VENDED[self._declaration.storage]
        if self._declaration.remote_signing and self._declaration.storage is StorageType.S3:
            allowed |= _SIGNER
        accepted: dict[str, str] = {}
        for key, value in config.items():
            if key in allowed and isinstance(value, str) and value:
                accepted[key] = value
            else:
                self._deny(Denial.KEY_NOT_ALLOWED)
        return accepted

    def properties(self, declared: Mapping[str, str], catalog: Mapping[str, str]) -> dict[str, str]:
        """The properties the catalog client is built with: what the catalog proposed, overridden by what the connection
        declares, and the FileIO implementation of the host."""
        return {**catalog, **declared, "py-io-impl": _FILE_IO[self._declaration.storage]}

    @staticmethod
    def _plausible(key: str, value: str) -> bool:
        if key == "prefix":
            return _PREFIX.fullmatch(value) is not None and clean_path(value)
        return 0 < len(value) <= _MAX_VALUE and all(ord(c) > 0x20 and ord(c) != 0x7F for c in value)

    # -- locations -------------------------------------------------------------------------------------------------

    def check_location(self, location: str) -> str:
        """`location` itself when it lies under a declared prefix. Otherwise the denial is counted and the attempt
        fails; the error does not say which location."""
        candidate = location if location.endswith("/") else location + "/"
        for prefix in self._declaration.warehouse_locations:
            if candidate.startswith(prefix) and clean_path(candidate[len(prefix) :]):
                return location
        raise self._refusal()

    def check_locations(self, locations: Iterable[str]) -> list[str]:
        """Every location, in order; the first one outside the declared prefixes stops the batch."""
        return [self.check_location(location) for location in locations]

    def accept_table(self, document: Mapping[str, object]) -> AcceptedTable:
        """A `LoadTable` response: every location it names must lie inside the warehouse, and its config is filtered.

        The locations are the table's own, the metadata file, the files that move the data (table properties), the
        manifest lists of the snapshots, the metadata log and the statistics files. A location that is missing or is
        not a string is refused: the confinement cannot be shown."""
        metadata = document.get("metadata")
        if not isinstance(metadata, Mapping):
            raise self._refusal()
        location = metadata.get("location")
        if not isinstance(location, str):
            raise self._refusal()
        self.check_location(location)
        self._check_named(document.get("metadata-location"), required=False)
        properties = metadata.get("properties", {})
        if not isinstance(properties, Mapping):
            raise self._refusal()
        for name in _LOCATION_PROPERTIES:
            self._check_named(properties.get(name), required=False)
        for entry, field in (
            ("snapshots", "manifest-list"),
            ("metadata-log", "metadata-file"),
            ("statistics", "statistics-path"),
            ("partition-statistics", "statistics-path"),
        ):
            for item in self._items(metadata.get(entry)):
                self._check_named(item.get(field), required=False)
        config = document.get("config", {})
        accepted = self.table_config(config if isinstance(config, Mapping) else {})
        return AcceptedTable(location, accepted)

    def _items(self, value: object) -> list[Mapping[str, object]]:
        if value is None:
            return []
        if not isinstance(value, list) or not all(isinstance(item, Mapping) for item in value):
            raise self._refusal()
        return [item for item in value if isinstance(item, Mapping)]

    def _check_named(self, value: object, *, required: bool) -> None:
        if value is None and not required:
            return
        if not isinstance(value, str):
            raise self._refusal()
        self.check_location(value)

    def _refusal(self) -> HygieneError:
        """The refusal of a location: counted, and carrying its code only."""
        self._deny(Denial.LOCATION_NOT_ALLOWED)
        return HygieneError(LOCATION_NOT_ALLOWED)

    # -- credentials -----------------------------------------------------------------------------------------------

    def require_explicit_credentials(self, properties: Mapping[str, str]) -> None:
        """Fails closed unless the properties hold credentials for the declared storage type. The host never falls back
        to an ambient chain (instance metadata, a profile, a workload identity)."""

        def has(*keys: str) -> bool:
            return all(properties.get(key) for key in keys)

        storage = self._declaration.storage
        if storage is StorageType.S3:
            present = has("s3.access-key-id", "s3.secret-access-key")
        elif storage is StorageType.GCS:
            present = has("gcs.oauth2.token")
        else:
            present = has("adls.connection-string") or (
                has("adls.account-name")
                and (
                    has("adls.account-key")
                    or has("adls.sas-token")
                    or has("adls.tenant-id", "adls.client-id", "adls.client-secret")
                )
            )
        if not present:
            self._deny(Denial.CREDENTIAL_MISSING)
            raise HygieneError(CREDENTIAL_UNAVAILABLE)

    def _deny(self, kind: Denial) -> None:
        self._denied[kind] += 1
