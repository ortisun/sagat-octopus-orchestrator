# Form model `form-model-v1` (draft)

DRAFT, not implemented. The rules by which `crates/infra-schema` turns a carrier schema (a version's
`inputSchema`, an approval `form.schema`) into the `FormModel` of
[openapi-fragment.yaml](./openapi-fragment.yaml). The walker is pure, deterministic and bounded; the
same version always yields the same model. The browser and the SDKs render the model and never
re-interpret the schema. Validation of a submitted input is always the run input gate and validator of
design 043, never the form model.

Revised on 2026-11-24 (research.md, Decisions 026-001 and 026-002): the form renders a subset of the
run-input subset of design 043 (`specs/043-run-input-validation/contracts/input-schema-subset.md`) and
reads its schema positions from that design's pre-check; hints moved out of the schema into
`inputFormHints` and an approval form's `hints`, because any `x-` keyword but `x-sagat-classification`
puts a schema outside the 043 subset.

## Walk

1. Resolve the carrier of the pinned version. No carrier: `UNAVAILABLE` / `NO_SCHEMA`.
2. Run the 043 pre-check on the carrier (cached by the run-input cache, keyed by tenant and digest). A
   refusal of the pre-check (a keyword outside the subset, a cycle, a pattern outside the allowlist, a
   limit) is `UNAVAILABLE` / `SCHEMA_UNUSABLE`: the start would be refused anyway
   (`RUN_INPUT_SCHEMA_UNUSABLE`), so the form never offers it. The walker reads the schema positions the
   pre-check recorded; it has no keyword list of its own beyond the table below.
3. Walk depth-first from the root object. Fields are ordered by the hint `order` ascending, then by
   property name: the stored canonical document sorts object members
   (`docs/architecture/workflow-ir-and-sdk-modes.md`), so declaration order is not available, and an
   author who cares about the order sets `order`.
4. A local `$ref` is followed once per location within the feature-016 expansion bound (1,000); `$defs`
   and `definitions` are containers only.
5. For each location compute the feature-016 effective level with the existing classification walker
   (one implementation; the form walker never computes levels itself).
6. Emit one `FormField` per declared leaf and one `group` field per nested object; arrays of a
   supported scalar type are `list` fields with `items` constraints.
7. Attach the hints of the carrier (`inputFormHints` of the definition, or `hints` of the approval
   `form`) by the field's instance pointer.

## Keyword mapping

Every keyword below is in the 043 subset. A keyword of that subset not listed here is not rendered.

| Schema | Field |
|---|---|
| `type: string` | `text`; `format: date` → `date`, `date-time` → `datetime`, `email` → `email`, `uri` → `url`; `maxLength > 200` → `textarea` unless a hint says otherwise. `format` only chooses the widget: the server never asserts it (043), so the field's help says the value is not checked against the format |
| `type: string` + `enum` | `select` (≤ 7 choices may be `radio` by hint) |
| `type: integer` / `number` | `number` with `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`; `multipleOf` (a positive integer in 043) is the control's step |
| `type: boolean` | `checkbox` |
| `type: [T, "null"]`, or `anyOf` / `oneOf` of exactly one supported schema and `{"type": "null"}` | the field of `T`, marked nullable: an empty control submits `null` (the shape Pydantic and Zod emit for an optional field) |
| `type: object` + `properties` | `group`; `minProperties` and `maxProperties` shown as constraints; `additionalProperties` (boolean or schema) is not rendered: extra keys are never offered |
| `type: array` + one scalar `items` | `list` with `minItems`/`maxItems` |
| `minLength`, `maxLength` | bounds of a text control, in Unicode code points as 043 counts them |
| `pattern` | shown as "must match the author's pattern"; the browser never evaluates it (its dialect is 043's allowlist, checked only by the server) |
| `const` | not rendered; included in the submission as the constant |
| `default` | prefilled, except for `secret` fields (never prefilled) |
| `title`, `description` | label and help text (rendered as plain text, never HTML) |
| `examples`, `$comment` | not rendered |
| hint `widget` | overrides the widget when compatible; incompatible hints are refused at publication (`FORM_HINT_INVALID`) |
| hint `placeholder` | placeholder text (plain) |
| effective level `secret` | `password`, write-only, `autocomplete=off`, never echoed |

Not rendered although inside the 043 subset: `allOf`; `anyOf` and `oneOf` other than the nullable form;
a type list other than `[T, "null"]`; arrays of objects; `items` holding a combinator. Such a field is
`supported: false` with `unsupportedKeyword`; on a required field (or an ancestor of one) the model is
`UNAVAILABLE` / `UNSUPPORTED_REQUIRED_FIELD`. Tuples are outside the 043 subset (Owner decision
2026-11-19-5), so they reach step 2 and are `SCHEMA_UNUSABLE`.

A location that feature 031 declares encoded (`x-sagat-encoding` in its draft) is treated as unsupported
with feature 031's reason `ENCODING_READ_BY_FORM` (the form is a read position of 031's encoded-field
registry); a hint on such a location is refused at publication with the same code.

## Hints

`inputFormHints` (root of the definition) and `form.hints` (an approval node) are objects from an instance
JSON pointer to `{widget, order, placeholder}`. The pointer is built only from names under `properties`
(the same shape as design 043's `details.field`), and must name a field of the carrier; any other pointer,
key or type is `FORM_HINT_INVALID` at publication and `validate`. Hints are not part of the schema: they
never change validation and never change the schema's digest. They are an IR addition behind the
instance-registry capability `form-hints-v1`.

## Other carriers

The walker takes any carrier pointer. Other features add a `FormModel.carrier` value additively when they
need a form for another declared schema (feature 027 renders an Update `argsSchema`). A carrier that
feature 016 does not classify (Query/Update schemas refuse classification annotations, FR-016-004)
yields `pii` for every field, the rule for unmarked values. A carrier outside the 043 subset is
`SCHEMA_UNUSABLE` whatever its kind.

## Bounds

Schema 64 KiB (the 043 limit as well); 128 fields by default (256 hard); nesting 4 (8 hard, below the 043
depth of 32); 100 choices per field (1,000 hard); exceeding any bound is `UNAVAILABLE` / `TOO_COMPLEX`.
Models are cached per process by `(version id, carrier pointer)` with the size bound of the feature-016
validator cache.

## Public links

`getPublicForm` returns the same model restricted to the link's exposed pointers. Presets are merged
by the server after submission and are never in the model. The model of a public link contains no
`default` of a field whose effective level is above `public`, so that an author's default for an
internal field is not shown to anonymous callers. A refusal of the merged input names a field only when
it lies inside an exposed pointer (research.md, Decision 026-017).

## Accessibility contract for renderers

Each field has a programmatic label (title or the property name), help text linked by
`aria-describedby`, its level as visible text, the required state, an error message linked to the field
after server validation, and keyboard operation for every widget. Errors are announced in a live region
and focus moves to the invalid field. The server reports the first violation only (design 043), so a form
with several invalid fields learns them one submission at a time.
