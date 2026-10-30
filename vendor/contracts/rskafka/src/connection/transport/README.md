# transport contract

## Overview

SASL credential/configuration plumbing, with configuration types re-exported through the public client API.

Source directory: [`vendor/rskafka/src/connection/transport`](../../../../../rskafka/src/connection/transport). This contract is outside the verified upstream snapshot.

## Interface

- `SaslConfig`, `Credentials`, `OauthCallback`, and `OauthBearerCredentials` are re-exported by rskafka::client. SaslConfig selects PLAIN, SCRAM-SHA-256, SCRAM-SHA-512, or OAUTHBEARER; Credentials::new constructs credentials. [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig] [vendor/rskafka/src/connection/transport/sasl.rs:Credentials] [vendor/rskafka/src/connection/transport/sasl.rs:OauthCallback] [vendor/rskafka/src/connection/transport/sasl.rs:OauthBearerCredentials] [vendor/rskafka/src/connection/transport/sasl.rs:Credentials::new] [vendor/rskafka/src/client/mod.rs:SaslConfig]
- get_sasl_config and mechanism are crate-internal configuration helpers. [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig::get_sasl_config] [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig::mechanism]

## Invariants

- OAUTHBEARER obtains its token asynchronously before building the synchronous rsasl callback; callback failures map to SaslError::Callback. [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig::get_sasl_config]
- OauthBearerCredentials Debug omits the callback/token source, but Credentials and SaslConfig derive Debug and are not credential-redacting formats. [vendor/rskafka/src/connection/transport/sasl.rs:OauthBearerCredentials] [vendor/rskafka/src/connection/transport/sasl.rs:Credentials] [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig]

## Side Effects

- Invokes the caller-supplied async OAuth token callback and constructs rsasl configuration; this source performs no network I/O itself. [vendor/rskafka/src/connection/transport/sasl.rs:SaslConfig::get_sasl_config]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```

This checks the upstream file set and hashes, allowing only the reviewed `zstd` manifest override. Do not run dependency-update commands inside the immutable snapshot. [scripts/check-vendor.py:verify]
