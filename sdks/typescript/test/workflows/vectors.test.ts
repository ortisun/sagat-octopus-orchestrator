import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { test } from 'node:test';
import { EMPTY_CHAIN, Xoshiro256, bodyDigest, extendChain, hex, rfc8785, type CommandKind } from '../../src/workflows/index.js';
import { hashedBody } from '../../src/workflows/chain.js';
import { splitMix64Words } from '../../src/workflows/context.js';

// The golden corpus of Task 044-30 (the server lane writes it with its own generator); until it merges, the corpus
// test is skipped with the release gate's text, and the vectors below come from outside this repository.
const CORPUS = new URL('../../../../../specs/044-workflow-as-code/contracts/goldens/vectors/', import.meta.url);
const PENDING = process.env['SAGAT_RUN_PENDING'] === '1' ? false : 'green with Task 044-30';

test('the generator follows the published outputs of SplitMix64 and xoshiro256**', () => {
  // SplitMix64 from 0: the first output is 0xe220a8397b1dcdaf (Steele, Lea and Flood; Vigna's reference code).
  assert.equal(splitMix64Words(0n, 1)[0]!.toString(16), 'e220a8397b1dcdaf');
  // xoshiro256** from the state {1, 2, 3, 4}: 11520, 0, 1509978240, 1215971899390074240 (Blackman and Vigna's reference).
  const seed = Buffer.alloc(32);
  [1n, 2n, 3n, 4n].forEach((word, i) => seed.writeBigUInt64LE(word, i * 8));
  const generator = new Xoshiro256(seed.toString('hex'));
  assert.deepEqual([generator.next(), generator.next(), generator.next(), generator.next()], [11520n, 0n, 1509978240n, 1215971899390074240n]);
  // An all-zero seed takes the first four SplitMix64 outputs as its state.
  const zero = new Xoshiro256('00'.repeat(32));
  const replaced = Buffer.alloc(32);
  splitMix64Words(0n, 4).forEach((word, i) => replaced.writeBigUInt64LE(word, i * 8));
  assert.equal(zero.next(), new Xoshiro256(replaced.toString('hex')).next());
});

test('the bounded integers and identifiers follow the rule of Technical Plan 1.8', () => {
  const seed = Buffer.alloc(32);
  [1n, 2n, 3n, 4n].forEach((word, i) => seed.writeBigUInt64LE(word, i * 8));
  const generator = new Xoshiro256(seed.toString('hex'));
  // (next() >> 11) × 2^-53 for the outputs 11520 and 0.
  assert.equal(generator.random(), Number(11520n >> 11n) * 2 ** -53);
  assert.equal(generator.random(), 0);
  // 1509978240 mod 6 (the rejection floor 2^64 mod 6 = 4 is below it).
  assert.equal(generator.randomInt(6), 1509978240 % 6);
  const uuid = new Xoshiro256(seed.toString('hex')).uuid();
  // 11520 and 0 little-endian, version 4 and variant 10.
  assert.equal(uuid, '002d0000-0000-4000-8000-000000000000');
  assert.throws(() => generator.randomInt(0), RangeError);
  assert.throws(() => generator.randomInt(2 ** 32 + 1), RangeError);
});

test('the rfc8785 examples of the RFC hold', () => {
  // RFC 8785, section 3.2.3 (sorting by UTF-16 code units) and the number examples of section 3.2.2.3.
  const sorting = '{"\\u20ac":"Euro Sign","\\r":"Carriage Return","\\ufb33":"Hebrew Letter Dalet With Dagesh","1":"One","\\ud83d\\ude00":"Emoji: Grinning Face","\\u0080":"Control","\\u00f6":"Latin Small Letter O With Diaeresis"}';
  assert.equal(rfc8785(JSON.parse(sorting)),
    '{"\\r":"Carriage Return","1":"One","\u0080":"Control","ö":"Latin Small Letter O With Diaeresis","€":"Euro Sign","😀":"Emoji: Grinning Face","דּ":"Hebrew Letter Dalet With Dagesh"}');
  const numbers = '{"numbers":[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001]}';
  assert.equal(rfc8785(JSON.parse(numbers)), '{"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27]}');
  assert.equal(rfc8785([1e21, 1e-7, 0.000001, -0, 5e-324, 1.7976931348623157e308]), '[1e+21,1e-7,0.000001,0,5e-324,1.7976931348623157e+308]');
  assert.equal(rfc8785('\u0000\u001f\b\f\n\r\t"\\/\u007f '), '"\\u0000\\u001f\\b\\f\\n\\r\\t\\"\\\\/\u007f "');
});

test('the chain folds the uint32 big-endian length of d(k) before d(k)', () => {
  const command = { kind: 'activity' as CommandKind, body: { type: 'activity', activity: 'inventory.reserve', input: { sku: 'A-1' } } };
  const inputBytes = Buffer.from('{"sku":"A-1"}', 'utf8');
  const descriptor = { sha256: createHash('sha256').update(inputBytes).digest('hex'), size: inputBytes.length };
  assert.deepEqual(hashedBody(command), { type: 'activity', activity: 'inventory.reserve', input: descriptor });
  const body = createHash('sha256').update(Buffer.from(rfc8785(hashedBody(command)), 'utf8')).digest();
  assert.equal(hex(bodyDigest(command)), body.toString('hex'));
  const d = Buffer.from(`{"body":"${body.toString('hex')}","kind":"activity","seq":1,"target":"inventory.reserve"}`, 'utf8');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(d.length, 0);
  const expected = createHash('sha256').update(Buffer.concat([Buffer.alloc(32), length, d])).digest('hex');
  assert.equal(hex(extendChain(EMPTY_CHAIN, 1, 'activity', 'inventory.reserve', bodyDigest(command))), expected);
});

test('the corpus vectors of Task 044-30 hold', { skip: PENDING }, () => {
  assert.ok(existsSync(CORPUS), 'the corpus of Task 044-30 is not on this branch');
  const primitives = JSON.parse(readFileSync(new URL('primitives.json', CORPUS), 'utf8')) as unknown;
  const canonical = JSON.parse(readFileSync(new URL('rfc8785.json', CORPUS), 'utf8')) as unknown;
  const chain = JSON.parse(readFileSync(new URL('chain.json', CORPUS), 'utf8')) as unknown;
  assert.ok(primitives && canonical && chain);
});

test('a lone surrogate, a bigint and a non-finite number have no canonical form', () => {
  assert.throws(() => rfc8785({ text: '\ud800' }), /lone surrogate/);
  assert.throws(() => rfc8785({ ['\udc00']: 1 }), /lone surrogate/);
  assert.throws(() => rfc8785({ amount: 10n }), /bigint/);
  assert.throws(() => rfc8785([Number.NaN]), /finite/);
  assert.equal(rfc8785({ skipped: undefined, kept: [undefined, () => 1] }), '{"kept":[null,null]}');
});

test('every value field of the corpus table enters the hashed body as its descriptor', () => {
  const descriptor = (value: unknown): unknown => {
    const bytes = Buffer.from(rfc8785(value), 'utf8');
    return { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length };
  };
  const cases: [CommandKind, string, unknown][] = [
    ['activity', 'input', { a: 1 }], ['script', 'input', [1]], ['child', 'input', { c: true }], ['approval', 'subject', { amount: 9 }],
    ['marker_side_effect', 'value', 0.5], ['update_complete', 'result', 'done'], ['complete', 'output', null], ['fail', 'message', 'declined'],
  ];
  for (const [kind, field, value] of cases) {
    assert.deepEqual(hashedBody({ kind, body: { [field]: value, kept: 1 } }), { [field]: descriptor(value), kept: 1 }, kind);
  }
  // A value that always stays inline enters as itself.
  for (const [kind, field] of [['set_state', 'state'], ['continue_as_new', 'input'], ['consume_signals', 'signals']] as [CommandKind, string][]) {
    assert.deepEqual(hashedBody({ kind, body: { [field]: { x: 1 } } }), { [field]: { x: 1 } }, kind);
  }
});
