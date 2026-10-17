# Database connections and OpenBao credentials

Feature 017 implementation is in progress. The catalog and credential management described below
are implemented. Managed database execution, probes, ad hoc runs, result disclosure and SDK database
builders are not yet delivered. See `specs/017-database-connections/tasks.md` for release gates.

## Configure a connection in Studio

1. Select the namespace and open **Connections → New connection**. Management requires
   `database:manage`; viewers can inspect nonsecret metadata.
2. Enter the logical name, bare hostname/IP, port and database. Keep certificate and hostname
   verification enabled; optionally add public CA certificates. Plaintext appears only for a
   development namespace and also needs explicit executor deployment permission when execution ships.
3. Choose an OpenBao secret, or **Create a secret** and enter the PostgreSQL username/password.
   The form creates an opaque logical secret containing `{username,password}` through Sagat's API.
   Passwords are not connection settings. The `requiredProvider: openbao` request prevents a local
   fallback when OpenBao has not been configured.
4. Select finite execution limits and Active or Paused, then save. Saving records settings; it does
   not test reachability or database privileges. Every edit, including a pause, creates a revision.
5. If credentials save but the connection fails, keep the form open and retry. The form clears the
   credential fields and retains the saved reference. An uncertain response is retried with the same
   request identity. A changed revision requires refreshing the current connection before editing.

Deletion creates a tombstone and retains history. It does not undo a customer database effect. The
current catalog ceiling is 100 connections per namespace; configurable ceilings/retention remain
pending. A historical revision returns its old settings alongside current lifecycle authority.

## Create, rotate or migrate a secret

Open **Secrets**. Administrators can create or rotate credentials; authorized developers see only
references allowed by their secret ACL. New values are never displayed again. PostgreSQL format
stores username/password together; the opaque format accepts bounded UTF-8 text for other consumers.

A legacy secret offers **Migrate**. Sagat reads/decrypts it on the backend, verifies the OpenBao
receipt and cuts over while preserving its name and ACL. The browser never reads the old value.
Migration requires the old keyring and OpenBao. After cutover, provider failure does not restore the
local authority. Rotating a stored credential does not update the PostgreSQL account itself.

Deployment variables, short-lived token-file requirements and receipt binding-key handling are in
`specs/017-database-connections/contracts/openbao.md`. Production bootstrap, cleanup, backup/restore
and runner network profiles are still delivery tasks; this guide does not substitute for them.

## Reproduce the catalog happy/failure journey

With Docker, the repository Rust toolchain, Node and frontend dependencies/Chromium installed:

```sh
cargo test -p sagat-tests --test integration_database_connections --locked
cargo test -p sagat-tests --test e2e_database_connections --locked -- --ignored --nocapture
```

The second command builds Studio, starts temporary PostgreSQL/OpenBao, then runs Chromium against
the real API. It intentionally loses an acknowledged connection response and verifies safe retry,
then checks reload, keyboard use and responsive accessibility. It uses temporary test identities,
not production tokens, and cleans up its services when the harness exits.
