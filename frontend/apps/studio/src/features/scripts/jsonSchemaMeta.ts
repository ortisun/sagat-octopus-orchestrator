/**
 * The JSON Schema meta-schema (draft-07, with the newer keywords the platform's validator knows
 * left open), bundled so the editor never has to fetch it: the page may only talk to its own
 * origin. Monaco's JSON service validates a schema document against it and underlines what is not a
 * schema (a `type` that is not a type, a `required` that is not a list of names). The platform
 * compiles the schema when the draft is saved and its answer is the one that counts.
 */
export const META_SCHEMA_URI = 'http://json-schema.org/draft-07/schema';

const nonNegativeInteger = { $ref: '#/definitions/nonNegativeInteger' };
const subschema = { $ref: '#' };
const subschemaMap = { type: 'object', additionalProperties: subschema, default: {} };

export const JSON_SCHEMA_META: Record<string, unknown> = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  $id: 'http://json-schema.org/draft-07/schema#',
  title: 'JSON Schema',
  definitions: {
    schemaArray: { type: 'array', minItems: 1, items: subschema },
    nonNegativeInteger: { type: 'integer', minimum: 0 },
    simpleTypes: { enum: ['array', 'boolean', 'integer', 'null', 'number', 'object', 'string'] },
    stringArray: { type: 'array', items: { type: 'string' }, uniqueItems: true, default: [] },
  },
  type: ['object', 'boolean'],
  properties: {
    $id: { type: 'string' },
    $schema: { type: 'string' },
    $ref: { type: 'string' },
    $comment: { type: 'string' },
    $anchor: { type: 'string' },
    title: { type: 'string' },
    description: { type: 'string' },
    default: true,
    readOnly: { type: 'boolean', default: false },
    writeOnly: { type: 'boolean', default: false },
    deprecated: { type: 'boolean', default: false },
    examples: { type: 'array', items: true },
    multipleOf: { type: 'number', exclusiveMinimum: 0 },
    maximum: { type: 'number' },
    exclusiveMaximum: { type: 'number' },
    minimum: { type: 'number' },
    exclusiveMinimum: { type: 'number' },
    maxLength: nonNegativeInteger,
    minLength: nonNegativeInteger,
    pattern: { type: 'string' },
    additionalItems: subschema,
    items: { anyOf: [subschema, { $ref: '#/definitions/schemaArray' }], default: true },
    prefixItems: { $ref: '#/definitions/schemaArray' },
    maxItems: nonNegativeInteger,
    minItems: nonNegativeInteger,
    uniqueItems: { type: 'boolean', default: false },
    contains: subschema,
    minContains: nonNegativeInteger,
    maxContains: nonNegativeInteger,
    maxProperties: nonNegativeInteger,
    minProperties: nonNegativeInteger,
    required: { $ref: '#/definitions/stringArray' },
    additionalProperties: subschema,
    unevaluatedProperties: subschema,
    unevaluatedItems: subschema,
    definitions: subschemaMap,
    $defs: subschemaMap,
    properties: subschemaMap,
    patternProperties: subschemaMap,
    dependencies: { type: 'object', additionalProperties: { anyOf: [subschema, { $ref: '#/definitions/stringArray' }] } },
    dependentRequired: { type: 'object', additionalProperties: { $ref: '#/definitions/stringArray' } },
    dependentSchemas: subschemaMap,
    propertyNames: subschema,
    const: true,
    enum: { type: 'array', items: true },
    type: {
      anyOf: [{ $ref: '#/definitions/simpleTypes' }, { type: 'array', items: { $ref: '#/definitions/simpleTypes' }, minItems: 1, uniqueItems: true }],
    },
    format: { type: 'string' },
    contentMediaType: { type: 'string' },
    contentEncoding: { type: 'string' },
    if: subschema,
    then: subschema,
    else: subschema,
    allOf: { $ref: '#/definitions/schemaArray' },
    anyOf: { $ref: '#/definitions/schemaArray' },
    oneOf: { $ref: '#/definitions/schemaArray' },
    not: subschema,
  },
  default: true,
};
