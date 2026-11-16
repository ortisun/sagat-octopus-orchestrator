# The run-input subset of JSON Schema (design 043)

The contract an author writes an `inputSchema` against, from release 0.3.0 (OD-2026-11-14-14, ADR-0090 decisions 5 and
6, ADR-0092 decision 3). A schema outside it is refused at publication in `enforce` mode and refuses every start of its
version (`RUN_INPUT_SCHEMA_UNUSABLE`); `control-plane input-schema-inventory` lists the stored ones. The refusal codes
are in `errors.md` beside this file. The limits, the charges and the budget below are part of the capability
`run-input-validation-v1`: a release that changes any of them so that an input or a schema accepted today is refused
ships a new capability, `run-input-validation-v2`, required the same way (design 043, Technical Plan 6; ADR-0092
decision 6). `the_limits_and_the_charges_of_run_input_validation_v1_are_pinned` (`crates/infra-schema/tests/run_input.rs`)
holds the limits, the budget and the units of one case per keyword family (D043-178).

## Dialects

A schema means the same when `$schema` is absent, draft-07 (`http://json-schema.org/draft-07/schema#`) or 2020-12
(`https://json-schema.org/draft/2020-12/schema`). Any other `$schema` is refused.

## Keywords

| Group | Keywords | What keeps the drafts equal |
|---|---|---|
| Structure | `type` (a name or a list of names), `properties`, `required`, `items` (one schema), `additionalProperties` (boolean or schema) | `items` as an array (draft-07 tuples) and `prefixItems` are refused; `integer` accepts a number with no fractional part |
| Values | `enum`, `const` | JSON values compared by value, numbers by mathematical value (`1` equals `1.0`) |
| Numbers | `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf` | the exclusive bounds are numbers (draft-04's boolean form is refused); `multipleOf` is a positive integer (floating-point remainders differ between implementations) |
| Lengths | `minLength`, `maxLength`, `minItems`, `maxItems`, `minProperties`, `maxProperties` | a string's length in Unicode code points |
| Pattern | `pattern` | the allowlist below |
| Combinators | `allOf`, `anyOf`, `oneOf` | each a non-empty array of schemas |
| References | `$ref` to a JSON pointer into the document (`#/...`), `$defs` and `definitions` as containers | the target is a schema position (not `#/enum/0`, `#/default`, `#/examples/0`, an `x-sagat-classification` value or a `properties` map itself); no cycle; no sibling keyword but annotations (draft-07 ignores siblings, 2020-12 applies them) |
| Ignored | `title`, `description`, `default`, `examples`, `$comment`, `format` (an annotation, never asserted); `$schema` and `$id` at the root only | the values of `enum`, `const`, `default`, `examples` and `x-sagat-classification` are data, not schemas |
| Platform | `x-sagat-classification` | the only `x-` keyword the platform reads; inert to validation |

Out of the subset: every other keyword, among them `patternProperties`, `uniqueItems`, `not`, `if`/`then`/`else`,
`contains`, `dependencies`, `dependentRequired`, `dependentSchemas`, `unevaluatedProperties`, `unevaluatedItems`,
`additionalItems`, `propertyNames`, `prefixItems`, `$dynamicRef`, `$recursiveRef`, `$anchor`, `$dynamicAnchor`, a nested
`$id` or `id`, a remote `$ref`, and any other `x-` keyword. One `inputSchema` serves both manual starts and the
envelope `{event, data}` of a trigger: a version started both ways writes an `anyOf` of the two shapes.

## The pattern allowlist

A `pattern` is parsed and checked node by node; its meaning is the same in ECMA-262 (which both drafts name) and in the
platform's engine. Matching is an unanchored search over code points.

Allowed:

- literal characters and escaped metacharacters; `\n`, `\r`, `\t`, `\xHH`;
- bracket classes of literals and ranges, negated too;
- `.`, compiled as any character but `\n`, `\r`, U+2028 and U+2029 (its ECMA-262 meaning);
- `^`, `$`, alternation, groups and non-capturing groups `(?:...)`;
- the repetitions `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, greedy or lazy, with bounds of at most 100.

Refused, with the reason:

- `\d`, `\D`, `\w`, `\W`, `\s`, `\S`, `\b`, `\B`, also inside brackets: Unicode classes in the engine, ASCII in
  ECMA-262; write `[0-9]` and the like;
- `\p{...}`, POSIX classes such as `[[:alpha:]]`, flags such as `(?i)`, `\A`, `\z`, named groups `(?P<...>` and
  `(?<...>`, backreferences and lookaround: not in one of the two readings;
- a class set operation (`&&`, `--`, `~~`) and a nested bracket: literals in ECMA-262;
- a `]` as the first item of a class (`[]a]`) and `[^]`: an empty class and any character in ECMA-262.

## Limits

| Limit | Value | Refused with |
|---|---|---|
| Schema size | 64 KiB of canonical bytes | `INPUT_SCHEMA_TOO_LARGE` (at publication `CLASSIFICATION_TOO_COMPLEX`, by feature 016's bound, first) |
| Schema depth | 32 (the root is 1) | `INPUT_SCHEMA_TOO_DEEP` (at publication `CLASSIFICATION_TOO_COMPLEX` first) |
| Application path from the root | 128 schema nodes | `INPUT_SCHEMA_TOO_DEEP` |
| A name of `properties` or `required` | 1 KiB | `INPUT_SCHEMA_NAME_TOO_LONG` |
| Patterns | 32, each at most a 16 KiB program and 512 NFA states, 256 KiB of programs in all; a repetition bound of at most 100 and groups and repetitions nested at most 64 deep | `INPUT_SCHEMA_PATTERN_UNSUPPORTED` |
| The divisor of `multipleOf` | a positive integer of at most 2^53 | `INPUT_SCHEMA_KEYWORD_UNSUPPORTED` |
| The JSON pointers of every schema position, together | 4 MiB (a long name high in the schema, repeated over thousands of positions below it, reaches it) | `INPUT_SCHEMA_TOO_LARGE`, at the root |
| The least work any input pays | below the budget | `INPUT_SCHEMA_TOO_COSTLY` |
| Input depth | 64 | the input: `RUN_INPUT_INVALID`, `rule` `maxDepth`, `field` `/` |
| Work of one validation | 1,000,000 units (`RUN_INPUT_WORK_UNITS`) | the input: `RUN_INPUT_INVALID`, `rule` `workLimit`, `field` `/` |

The least work is a lower bound the pre-check computes from charges that do not depend on the input: one unit per
application and per `$ref` followed, per `type` name, per `enum` member, one for `const`; the sum over `allOf`, the
cheapest `anyOf` branch, and for `oneOf` its cheapest branch plus two units per other branch.

## The charges

Every loop step and every subschema application costs at least one unit:

- one unit per subschema application (an empty schema included) and per `$ref` followed;
- one unit per name of `required`, per name of a `type` list, per branch of a combinator (beside the branch's own
  application);
- for each instance member, its `properties` lookup at `log2(n) × (1 + length / 64)` units and one unit for its
  `additionalProperties` decision; one unit per array element for `items`;
- for `enum`, one unit per member plus the smaller of the member's and the value's size; for `const`, the smaller of the
  two sizes;
- for `multipleOf` on an integral float, the steps of an exact remainder, logarithmic in the exponent;
- for a length check, the length in code points; for a `pattern`, (length in bytes + 1) × NFA states, charged before the
  match. Within one subschema the length checks run first, so a long string fails on its length: pair `pattern` with
  `maxLength`.

A charge that would pass the budget stops the whole validation before the step, with `workLimit`; it is never the
failure of one branch, so `oneOf` of a branch under the budget and a branch over it is `workLimit`, never an acceptance.

## The refusal

The first violation is answered with a closed `rule`, the keyword that failed (or `workLimit`, `maxDepth`, `other`), and
a pointer built only from members matched through `properties` and indices matched through `items`; a member reached
through `additionalProperties`, whose key the caller chose, adds no segment, and a `required` violation adds the missing
name, which the schema holds. No value of the input is ever formatted. A start whose input the caller did not send (a
rerun, a batch rerun, a reset, a pending reset, a trigger-now, a private rerun) answers `/` and `other`.
