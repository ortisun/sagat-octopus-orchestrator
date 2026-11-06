"""The REST catalog of the host (feature 035: T035-018, D035-050, D035-094, G035-041, G035-043).

PyIceberg's client trusts what the catalog says. It merges the ``defaults`` and ``overrides`` of ``/v1/config`` over its
own properties, which lets a catalog name the class that builds the file IO, move the endpoint, the proxy and the
signer, and change the ``uri`` the client talks to. And it builds the file IO of a table from the table's own properties
overridden by the config and the storage credentials of the response. It also sends every request without a time limit.

``LakeRestCatalog`` is PyIceberg's client with three changes, each at the one place where an answer would otherwise be
used before anyone looked at it:

* ``/v1/config`` is read through ``Hygiene.catalog_config`` (only ``warehouse`` and ``prefix`` are taken) and the result
  is overridden by what the connection declares, with the file IO pinned (``Hygiene.properties``);
* a table's response is read through ``Hygiene.accept_table`` and ``storage_credentials`` before ``_response_to_table``
  builds its file IO: the locations must lie in the warehouse, the table's properties pass the allowlist, the config and
  the vended credentials keep only the keys of the declared storage;
* every request of the session has a time limit (``request_timeout``) unless the caller gives its own.

``_fetch_config`` is a copy of PyIceberg 0.12.0's with the filtering, and ``_response_to_table`` and ``_create_session``
are extended: the version is pinned in ``requirements.lock``, and ``tests/test_rest_catalog.py`` shows the client of
PyIceberg following a lying catalog, so an upgrade that changes either is noticed.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING, Any, cast
from urllib.parse import unquote

from pyiceberg.catalog import URI, WAREHOUSE_LOCATION
from pyiceberg.catalog.rest import (
    DEFAULT_ENDPOINTS,
    DEFAULT_NAMESPACE_SEPARATOR,
    NAMESPACE_SEPARATOR_PROPERTY,
    VIEW_ENDPOINTS,
    VIEW_ENDPOINTS_SUPPORTED,
    VIEW_ENDPOINTS_SUPPORTED_DEFAULT,
    ConfigResponse,
    Endpoints,
    RestCatalog,
)
from pyiceberg.catalog.rest.response import _handle_non_200_response
from pyiceberg.catalog.rest.scan_planning import StorageCredential
from pyiceberg.utils.properties import property_as_bool
from requests import HTTPError

from .config_hygiene import Hygiene

if TYPE_CHECKING:
    from pyiceberg.catalog.rest import TableResponse
    from pyiceberg.table import Table
    from requests import Session


class LakeRestCatalog(RestCatalog):
    """PyIceberg's REST client that believes nothing the catalog says about itself, its tables or its credentials."""

    def __init__(self, name: str, *, hygiene: Hygiene, request_timeout: float, **properties: str) -> None:
        if not request_timeout > 0:
            raise ValueError("a request timeout is positive")
        self._hygiene = hygiene
        self._request_timeout = request_timeout
        super().__init__(name, **properties)

    def _create_session(self) -> Session:
        session = super()._create_session()
        _bound(session, self._request_timeout)
        return session

    def _fetch_config(self) -> None:
        params = {}
        if warehouse_location := self.properties.get(WAREHOUSE_LOCATION):
            params[WAREHOUSE_LOCATION] = warehouse_location

        with self._create_session() as session:
            response = session.get(self.url(Endpoints.get_config, prefixed=False), params=params)
        try:
            response.raise_for_status()
        except HTTPError as exc:
            _handle_non_200_response(exc, {})
        config_response = ConfigResponse.model_validate_json(response.text)

        # PyIceberg does `config = defaults; config.update(self.properties); config.update(overrides)`, so the catalog's
        # overrides win over the client's own properties
        defaults, overrides = config_response.defaults or {}, config_response.overrides or {}
        accepted = self._hygiene.catalog_config(defaults, overrides)
        config = self._hygiene.properties(self.properties, accepted)
        self.properties = config
        self.uri = config[URI]

        endpoints = config_response.endpoints
        if endpoints:
            self._supported_endpoints = set(endpoints)
        else:
            self._supported_endpoints = set(DEFAULT_ENDPOINTS)
            if property_as_bool(self.properties, VIEW_ENDPOINTS_SUPPORTED, VIEW_ENDPOINTS_SUPPORTED_DEFAULT):
                self._supported_endpoints.update(VIEW_ENDPOINTS)

        separator_from_properties = self.properties.get(NAMESPACE_SEPARATOR_PROPERTY, DEFAULT_NAMESPACE_SEPARATOR)
        if not separator_from_properties:
            raise ValueError("Namespace separator cannot be an empty string")
        self._namespace_separator = unquote(separator_from_properties)

    def _response_to_table(self, identifier_tuple: tuple[str, ...], table_response: TableResponse) -> Table:
        document = table_response.model_dump(mode="json", by_alias=True, exclude_none=True)
        accepted = self._hygiene.accept_table(document)
        offered = [
            {"prefix": credential.prefix, "config": credential.config}
            for credential in table_response.storage_credentials
        ]
        credentials = [
            StorageCredential(prefix=cast(str, entry["prefix"]), config=cast(dict[str, str], entry["config"]))
            for entry in self._hygiene.storage_credentials(offered)
        ]
        filtered = table_response.model_copy(
            update={
                "config": accepted.config,
                "storage_credentials": credentials,
                "metadata": table_response.metadata.model_copy(update={"properties": accepted.properties}),
            }
        )
        return super()._response_to_table(identifier_tuple, filtered)


def _bound(session: Session, seconds: float) -> None:
    """Gives every request of ``session`` a time limit, unless the caller names one."""
    send: Callable[..., Any] = session.request

    def request(method: str, url: str, **kwargs: Any) -> Any:
        if kwargs.get("timeout") is None:
            kwargs["timeout"] = seconds
        return send(method, url, **kwargs)

    setattr(session, "request", request)  # noqa: B010
