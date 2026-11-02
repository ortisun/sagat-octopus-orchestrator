# Form model `form-model-v1` (draft)

DRAFT, not implemented. The rules by which `crates/infra-schema` turns a carrier schema (a version's
`inputSchema`, an approval `form.schema`) into the `FormModel` of
[openapi-fragment.yaml](./openapi-fragment.yaml). The walker is pure, deterministic and bounded; the
same version always yields the same model. The browser and the SDKs render the model and never
re-interpret the schema. Validation of a submitted input is always the server's ordinary schema
validation, never the form model.

## Walk

1. Resolve the carrier of the pinned version. No carrier: `UNAVAILABLE` / `NO_SCHEMA`.
2. Walk depth-first from the root object. Fields are ordered by `x-sagat-form.order` ascending, then
   by property name: the stored canonical document sorts object members
   (`docs/architecture/workflow-ir-and-sdk-modes.md`), so declaration order is not available, and an
   author who cares about the order sets `order`.
3. A local `$ref` is followed once per location within the feature-016 expansion bound (1,000).
4. For each location compute the feature-016 effective level with the existing classification walker
   (one implementation; the form walker never computes levels itself).
5. Emit one `FormField` per declared leaf and one `group` field per nested object; arrays of a
   supported scalar type are `list` fields with `items` constraints.

## Keyword mapping

| Schema | Field |
|---|---|
| `type: string` | `text`; `format: date` → `date`, `date-time` → `datetime`, `email` → `email`, `uri` → `url`; `maxLength > 200` → `textarea` unless a hint says otherwise |
| `type: string` + `enum` | `select` (≤ 7 choices may be `radio` by hint) |
| `type: integer` / `number` | `number` with bounds |
| `type: boolean` | `checkbox` |
| `type: object` + `properties` | `group`; `additionalProperties` other than `false` is ignored for rendering (extra keys are never offered) |
| `type: array` + scalar `items` | `list` with `minItems`/`maxItems` |
| `const` | not rendered; included in the submission as the constant |
| `default` | prefilled, except for `secret` fields (never prefilled) |
| `title`, `description` | label and help text (rendered as plain text, never HTML) |
| `x-sagat-form.widget` | overrides the widget when compatible; incompatible hints are refused at publication |
| `x-sagat-form.placeholder` | placeholder text (plain) |
| effective level `secret` | `password`, write-only, `autocomplete=off`, never echoed |

Unsupported: `oneOf`, `anyOf`, `allOf` with differing branches, `if/then/else`, `not`, `patternProperties`,
`dependentSchemas`, `prefixItems`, arrays of objects, `contentEncoding`, remote `$ref`, and any keyword
not listed above. An unsupported keyword marks its field `supported: false` with `unsupportedKeyword`;
on a required field (or an ancestor of one) the model is `UNAVAILABLE` / `UNSUPPORTED_REQUIRED_FIELD`.
A location that feature 031 declares encoded (`x-sagat-encoding` in its draft) is treated as unsupported
with feature 031's reason `ENCODING_READ_BY_FORM` (the form is a read position of 031's encoded-field
registry); an `x-sagat-form` hint on such a location is refused at publication with the same code.

## Other carriers

The walker takes any carrier pointer. Other features add a `FormModel.carrier` value additively when they
need a form for another declared schema (feature 027 renders an Update `argsSchema`). A carrier that
feature 016 does not classify (Query/Update schemas refuse classification annotations, FR-016-004)
yields `pii` for every field, the rule for unmarked values.

## Bounds

Schema 64 KiB; 128 fields by default (256 hard); nesting 4 (8 hard); 100 choices per field (1,000
hard); exceeding any bound is `UNAVAILABLE` / `TOO_COMPLEX`. Models are cached per process by
`(version id, carrier pointer)` with the size bound of the feature-016 validator cache.

## Public links

`getPublicForm` returns the same model restricted to the link's exposed pointers. Presets are merged
by the server after submission and are never in the model. The model of a public link contains no
`default` of a field whose effective level is above `public`, so that an author's default for an
internal field is not shown to anonymous callers.

## Accessibility contract for renderers

Each field has a programmatic label (title or the property name), help text linked by
`aria-describedby`, its level as visible text, the required state, an error message linked to the field
after server validation, and keyboard operation for every widget. Errors are announced in a live region
and focus moves to the first invalid field.
