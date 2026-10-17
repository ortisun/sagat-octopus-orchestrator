# Database connections and OpenBao credentials

Configure a namespace connection once and reuse it through the five SDKs, Studio workflows,
ad hoc steps/workflows and probes. Credentials are stored in OpenBao and resolved only for an
authorized managed attempt. See [execution](../guides/managed-database-executions.md),
[SDK examples](../guides/managed-database-sdk.md) and the [delivery evidence](../../specs/017-database-connections/verification.md).

## Configure a connection in Studio

1. Select the namespace and open **Connections → New connection**. Management requires
   `database:manage`; viewers can inspect nonsecret metadata.
2. Enter the logical name, bare hostname/IP, port and database. Keep certificate and hostname
   verification enabled; optionally add public CA certificates. Plaintext appears only for a
   development namespace and also needs explicit executor deployment permission.
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
default catalog ceiling is 100 connections per namespace (operator maximum 1,000); existing
quota governance controls catalog, concurrency, pending intents and operation limits. A historical revision returns its old settings alongside current lifecycle authority.

## Create, rotate or migrate a secret

Open **Secrets**. Administrators can create or rotate credentials; authorized developers see only
references allowed by their secret ACL. New values are never displayed again. PostgreSQL format
stores username/password together; the opaque format accepts bounded UTF-8 text for other consumers.

A legacy secret offers **Migrate**. Sagat reads/decrypts it on the backend, verifies the OpenBao
receipt and cuts over while preserving its name and ACL. The browser never reads the old value.
Migration requires the old keyring and OpenBao. After cutover, provider failure does not restore the
local authority. Rotating a stored credential does not update the PostgreSQL account itself.

Deployment variables, short-lived token-file requirements and receipt binding-key handling are in
`specs/017-database-connections/contracts/openbao.md`. See [production bootstrap/cleanup/network profiles](openbao.md) and
[rotation/migration/backup/restore](database-recovery.md) before operating the integration.

## Reproduce the catalog happy/failure journey

With Docker, the repository Rust toolchain, Node and frontend dependencies/Chromium installed:

```sh
cargo build -p sagat-database-runner --bin database-runner --locked
cargo test -p sagat-tests --test integration_database_connections --locked
cargo test -p sagat-tests --test e2e_database_connections --locked -- --ignored --nocapture
```

The second command builds Studio, starts temporary PostgreSQL/OpenBao, then runs Chromium against
the real API. It intentionally loses an acknowledged connection response and verifies safe retry,
then checks reload, keyboard use and responsive accessibility. It uses temporary test identities,
not production tokens, and cleans up its services when the harness exits.
