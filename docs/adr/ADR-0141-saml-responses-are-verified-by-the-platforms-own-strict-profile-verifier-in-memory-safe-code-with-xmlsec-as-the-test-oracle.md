# ADR-0141: SAML responses are verified by the platform's own strict-profile verifier, in memory-safe code, with xmlsec as the test oracle

**Status:** Proposed
**Decided by:** agent proposal (design 013, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 013 is complete
**Refines / supersedes:** none. Follows the pattern of Architecture decision 0090 and design 043 (the platform's own bounded validator for a documented subset, with an established library as the oracle in tests)
**Enforced by:** planned in design 013 and not existing yet: the corpus `tests/security/federated-login/saml/ (target `security_federated_login`)` (signature wrapping in each known placement, a second assertion, a comment inside the name identifier, a DTD and an entity, an unsigned assertion in a signed response, SHA-1, a reference to another element, an expired or not-yet-valid assertion, a wrong audience, recipient or destination, a replayed assertion identifier, an unsolicited response) with each case refused by its named reason; the oracle test that, for every accepted fixture and for generated documents in the profile, `xmlsec1 --verify` agrees with the platform's verifier on the signature verdict; the architecture rule that no crate of the server depends on a C XML library

## Context

Design 013 lets a tenant sign in through a SAML 2.0 identity provider. A SAML response is an XML document, posted by
the browser, whose trust rests on an XML signature. SAML verification has a long record of serious defects in widely
used libraries: XML signature wrapping (the signature covers one element and the application reads another),
comment and canonicalization tricks in the name identifier, entity expansion, and parser memory-safety bugs. The usual
Rust option binds `libxmlsec1` and `libxml2` (C) into the process that holds the database credentials. The control
plane is that process.

What a service provider needs from a response is narrow: one assertion, signed by a known key, for this audience and
this request, within its time window, naming one subject and a few attributes. Everything else the XML signature
standard allows (other transforms, other canonicalizations, references outside the document, multiple signatures,
encrypted keys) can be refused.

## Decision

1. The server MUST verify SAML responses with the platform's own verifier for a documented strict profile, written in
   safe Rust, in the adapter crate of design 013 (`crates/infra-federation`). No crate of the server MAY link a C XML
   or XML signature library.
2. The parser MUST refuse a document type declaration, an entity, a processing instruction other than the XML
   declaration, a document over 256 KiB, a depth over 64 and more than 10,000 elements, before any signature work.
3. The profile MUST accept exactly one `Assertion` (an `EncryptedAssertion` is refused), signed either itself or by a
   signature on the enclosing `Response` (whose direct child it then is), with at most one `Signature` per element,
   each an immediate child of the element it signs, with one `Reference` whose URI is `#` plus that element's `ID`; when
   both are signed, both MUST verify. Transforms: the enveloped-signature transform and exclusive canonicalization
   only (the `#WithComments` forms refused by name); SHA-256 digests; RSA (at least 2,048 bits) with SHA-256 or ECDSA
   P-256 with SHA-256, by a key configured on the connection only (`KeyInfo` and embedded certificates are ignored).
   Every `ID` in the document MUST be unique. The element whose signature verified MUST be the element the platform
   reads, found by identity, never by a second search of the document.
4. The verifier MUST check, with 60 seconds of tolerance on times, over the verified Assertion: its `Issuer` equals
   the connection's identity provider; the audience equals the platform's entity identifier for the connection;
   `NotBefore` and `NotOnOrAfter`, the latter at most 10 minutes ahead so that the replay store is bounded; an
   `AuthnStatement` is present; the subject confirmation `Method` is `bearer`, its data has `Recipient` equal to the
   connection's assertion consumer service, `InResponseTo` equal to a pending request of this browser, `NotOnOrAfter`
   and no `NotBefore`; and the assertion identifier has not been seen before its expiry (a replay store). It MAY read
   exactly these unsigned values of the Response, compared by equality and used for nothing else: `StatusCode`
   (`Success`), `Destination` (the assertion consumer service), `InResponseTo` (equal to the subject confirmation's)
   and `Issuer` when present (equal to the Assertion's). The request body MUST be bounded before base64 decoding.
5. The name identifier and the attributes MUST be read as text content of the verified element only; a comment or a
   processing instruction inside them MUST refuse the response.
6. The exclusive canonicalization MUST be the platform's implementation, held to `xmlsec1` as an oracle in the test
   suite (the oracle runs only in tests, in a disposable container).
7. An unsolicited (identity-provider-initiated) response MUST NOT sign anyone in; the consumer service MAY answer it by
   starting a service-provider-initiated sign-in through the same connection.

## Considered options

1. **The platform's strict-profile verifier in safe Rust, xmlsec as the oracle (chosen).** The attack surface is the
   profile, not the standard; no C parser runs beside the database credentials; the oracle catches a
   canonicalization error before release.
2. **`samael` (bindings to `xmlsec1` and `libxml2`).** Complete and used. Rejected: C code in the control plane
   parsing attacker input, and the whole standard reachable unless the application adds the same profile checks on
   top, which it must do anyway.
3. **A separate verifier process (a sidecar with `xmlsec1`).** Isolates the C code. Rejected: a new process to
   operate, and a constitution that splits processes only after measurement (principle II); the profile checks would
   still be needed in the control plane.
4. **No SAML, OIDC only.** Least risk. Rejected: many enterprises, and many identity providers in regulated sectors,
   offer only SAML; parity with Temporal Cloud and Windmill's enterprise sign-in includes it.

## Consequences

- The platform maintains a canonicalization implementation; its scope is the profile, and the oracle test bounds the
  cost of a mistake.
- Encrypted assertions are not supported in 0.3.0 (Gap 013-004 of design 013); the response travels over TLS and is
  consumed by the platform only.
- A connection whose provider signs with SHA-1, or with an RSA key under 2,048 bits, cannot be activated until the
  provider changes; the certificate's own signature algorithm does not matter for a key the connection pins.
- Rollback: disabling SAML connections leaves OIDC connections working.

## Evidence

The risk record is public (signature wrapping, canonicalization and comment-truncation advisories in SAML libraries
of several languages). The planned corpus and oracle are listed under **Enforced by**; none exists yet.
