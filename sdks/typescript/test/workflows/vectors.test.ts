import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { EMPTY_CHAIN, Xoshiro256, bodyDigest, extendChain, hex, rfc8785, type CommandKind } from '../../src/workflows/index.js';
import { commandDigestInput, hashedBody } from '../../src/workflows/chain.js';
import { splitMix64Words } from '../../src/workflows/context.js';

// The golden corpus of Task 044-30, written by the server lane's own generator (`generate.mjs`), never by this SDK.
const CORPUS = new URL('../../../../../specs/044-workflow-as-code/contracts/goldens/vectors/', import.meta.url);

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

type Json = Record<string, unknown>;
const readVectors = (name: string): Json => JSON.parse(readFileSync(new URL(name, CORPUS), 'utf8')) as Json;
const list = (value: unknown, what: string): Json[] => {
  // Each file must yield vectors, so that this test cannot pass on an empty corpus.
  assert.ok(Array.isArray(value) && value.length > 0, `${what} holds no vector`);
  return value as Json[];
};

/** The runtime's model of a chain vector's command: a work command's value beside its fragment, a typed body's values as themselves. */
function modelOf(vector: Json): { kind: CommandKind; body: Record<string, unknown>; sagaScope?: string } {
  const kind = vector['kind'] as CommandKind;
  const typed = (vector['typed'] ?? {}) as Record<string, Json>;
  const field = { activity: 'input', script: 'input', child: 'input', approval: 'subject' }[kind as string];
  let body: Record<string, unknown>;
  if (vector['fragment']) {
    body = { ...(vector['fragment'] as Json) };
    if (field) body[field] = vector['value'];
  } else {
    switch (kind) {
      case 'timer_cancel': case 'step_cancel': body = { target: typed['target']!['target'] }; break;
      case 'saga_open': {
        const open = typed['sagaOpen']!;
        body = { scope: open['scope'], parentScope: open['parentScope'] ?? null, onCompensationFailure: open['onCompensationFailure'], cancelPolicy: open['cancelPolicy'] };
        break;
      }
      case 'saga_close': body = { scope: typed['sagaClose']!['target'] }; break;
      case 'saga_abort': body = { scope: typed['sagaAbort']!['target'] }; break;
      case 'marker_patch': body = { markerId: typed['marker']!['markerId'] }; break;
      case 'marker_side_effect': body = { markerId: typed['marker']!['markerId'], value: typed['marker']!['sideEffectValue'] }; break;
      case 'set_state': body = { state: typed['setState'] }; break;
      case 'consume_signals':
        body = { signals: (typed['consumeSignals']!['signals'] as Json[]).map((s) => ({ name: s['signalName'], seqs: (s['seqs'] as string[]).map(Number) })) };
        break;
      case 'update_accept': case 'update_reject': {
        const decision = typed['updateDecision']!;
        body = { admittedSeq: Number(decision['admittedSeq']), accepted: decision['accepted'] === true };
        if (decision['accepted'] !== true) body['reason'] = decision['rejectionReason'];
        break;
      }
      case 'update_complete': {
        const result = typed['updateResult']!;
        body = 'result' in result ? { admittedSeq: Number(result['admittedSeq']), result: result['result'] } : { admittedSeq: Number(result['admittedSeq']), failureCode: result['failureCode'] };
        break;
      }
      case 'complete': body = { output: typed['complete'] }; break;
      case 'fail': body = { code: typed['fail']!['code'], message: typed['fail']!['safeMessage'] }; break;
      case 'continue_as_new': {
        const next = typed['continueAsNew']!;
        body = { input: next['input'], state: next['state'] ?? null, version: next['activeVersion'] === true ? 'active' : 'same' };
        break;
      }
      case 'acknowledge_cancellation': body = {}; break;
      default: throw new Error(`no model for the kind ${kind}`);
    }
  }
  return vector['sagaScope'] === undefined ? { kind, body } : { kind, body, sagaScope: vector['sagaScope'] as string };
}

test('the RFC 8785 cases of the corpus hold', () => {
  for (const vector of list(readVectors('rfc8785.json')['cases'], 'rfc8785.json')) {
    const canonical = rfc8785(JSON.parse(vector['input'] as string));
    assert.equal(canonical, vector['canonical'], vector['name'] as string);
    assert.equal(Buffer.from(canonical, 'utf8').toString('hex'), vector['canonicalHex'], vector['name'] as string);
  }
});

test('the primitive streams of the corpus hold', () => {
  const vectors = readVectors('primitives.json');
  for (const seed of list(vectors['seeds'], 'primitives.json')) {
    const label = seed['label'] as string;
    const fresh = new Xoshiro256(seed['seedHex'] as string);
    const next = seed['next'] as string[];
    assert.deepEqual(next.map(() => fresh.next().toString(16).padStart(16, '0')), next, `${label}: next`);
    // One stream in the order the file states: random, then randomInt for each n, then uuid.
    const stream = new Xoshiro256(seed['seedHex'] as string);
    assert.deepEqual((seed['random'] as number[]).map(() => stream.random()), seed['random'], `${label}: random`);
    for (const draw of seed['randomInt'] as Json[]) {
      const values = draw['values'] as number[];
      assert.deepEqual(values.map(() => stream.randomInt(draw['n'] as number)), values, `${label}: randomInt(${String(draw['n'])})`);
    }
    assert.deepEqual((seed['uuid'] as string[]).map(() => stream.uuid()), seed['uuid'], `${label}: uuid`);
  }
  for (const rejection of list(vectors['rejection'], 'primitives.json rejection')) {
    assert.equal(new Xoshiro256(rejection['seedHex'] as string).randomInt(rejection['n'] as number), rejection['value']);
  }
});

test('every command of the corpus chain vectors hashes to its body, d(k) and H(k)', () => {
  let compared = 0;
  for (const chainList of list(readVectors('chain.json')['lists'], 'chain.json')) {
    let chain = EMPTY_CHAIN;
    for (const vector of list(chainList['commands'], String(chainList['name']))) {
      const where = `${String(chainList['name'])} command ${String(vector['seq'])} (${String(vector['kind'])})`;
      const command = modelOf(vector);
      assert.deepEqual(hashedBody(command), vector['body'], `${where}: the hashed body`);
      const digest = bodyDigest(command);
      assert.equal(hex(digest), vector['bodySha256'], `${where}: the body digest`);
      assert.equal(hex(commandDigestInput(vector['seq'] as number, command.kind, vector['target'] as string, digest)), vector['dHex'], `${where}: d(k)`);
      chain = extendChain(chain, vector['seq'] as number, command.kind, vector['target'] as string, digest);
      assert.equal(hex(chain), vector['chainHex'], `${where}: H(k)`);
      compared++;
    }
  }
  assert.ok(compared >= 30, `only ${compared} commands compared`);
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

test('every identifier is a version 4 UUID with the variant bits 10', () => {
  const generator = new Xoshiro256('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f');
  for (let i = 0; i < 256; i++) assert.match(generator.uuid(), /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});
