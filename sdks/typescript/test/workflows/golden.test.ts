import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { Replayer, eventFromProto3Json, eventToProto3Json, type ReplayVerdict } from '../../src/workflows/index.js';
import { blobRefsOf, eventFromWire } from '../../src/workflows/protocol.js';
import { listScenarioRuns, loadGoldenRun } from '../fake-workflow-server.js';
import { NONDETERMINISM_CHANGED, PURCHASE_V2, REFERENCE_RUNS, commandsOf, historyLinesOf, type ReferenceRun } from './goldens.js';
import { bundleOf } from './support.js';

// The golden corpus of Task 044-30 replayed by this SDK: every visible history of every scenario run is read through
// the protocol adapter (Decision 044-320), and replayed against the reference functions of `goldens.ts`, whose chain
// must be the corpus's (Decision 044-317).

function exportOf(reference: ReferenceRun): string[] {
  const commands = commandsOf(reference.scenario, reference.run);
  const header = {
    exportFormat: 1, seed: commands.seedHex, gatedEntries: commands.lease?.gatedEntries ?? [],
    stateMaxBytes: commands.lease?.stateMaxBytes ?? reference.stateMaxBytes ?? 0,
  };
  return [JSON.stringify(header), ...historyLinesOf(reference.scenario, reference.run)];
}

function replay(reference: ReferenceRun, source = latestVersion(reference)): Promise<ReplayVerdict> {
  const commands = commandsOf(reference.scenario, reference.run);
  return Replayer.replay(exportOf(reference), { bundleCode: bundleOf({ [commands.workflowType]: source }).code, workflowType: commands.workflowType });
}

function latestVersion(reference: ReferenceRun): ReferenceRun['versions'][string] {
  const commands = commandsOf(reference.scenario, reference.run);
  const completed = commands.activations.filter((activation) => activation.window.throughSeq !== null && activation.failedAttempts === undefined);
  const version = `v${completed[completed.length - 1]?.codeVersion ?? 1}`;
  const source = reference.versions[version];
  assert.ok(source, `${reference.scenario}/${reference.run} has no reference function for ${version}`);
  return source;
}

test('every scenario run of the corpus has a reference function', () => {
  const runs = listScenarioRuns().map(({ scenario, run }) => `${scenario}/${run}`).sort();
  assert.deepEqual(REFERENCE_RUNS.map(({ scenario, run }) => `${scenario}/${run}`).sort(), runs);
});

test('every history line of the corpus maps to the model, the same from the wire, and the model survives its own export line', () => {
  let events = 0;
  for (const { scenario, run } of REFERENCE_RUNS) {
    const lines = historyLinesOf(scenario, run);
    // The same events through the gRPC loader's decoding (the form a worker receives) map to the same model.
    const wire = loadGoldenRun(scenario, run).history;
    assert.equal(wire.length, lines.length);
    lines.forEach((line, index) => {
      const json = JSON.parse(line) as Record<string, unknown>;
      const event = eventFromProto3Json(json);
      assert.deepEqual(eventFromWire(wire[index]!), event, `${scenario}/${run} line ${index + 1}`);
      assert.deepEqual(eventFromProto3Json(eventToProto3Json(event)), event, `${scenario}/${run} line ${index + 1} written back`);
      events++;
    });
    assert.deepEqual(blobRefsOf(wire), [], `${scenario}/${run}: the corpus's histories reference no blob`);
  }
  assert.ok(events > 1000, `only ${events} events read`);
});

for (const reference of REFERENCE_RUNS) {
  test(`the corpus history of ${reference.scenario}/${reference.run} replays with its chain`, async () => {
    const commands = commandsOf(reference.scenario, reference.run);
    const completed = commands.activations.filter((activation) => activation.window.throughSeq !== null && activation.failedAttempts === undefined);
    const last = completed[completed.length - 1]!;
    const verdict = await replay(reference);
    assert.deepEqual(verdict, { verdict: 'MATCH', activations: completed.length, commands: last.commandCountAfter, chain: last.chainAfterHex });
  });
}

test('the changed code of code-nondeterminism replays to the mismatch the corpus reports', async () => {
  const reference = REFERENCE_RUNS.find((run) => run.scenario === 'code-nondeterminism')!;
  const verdict = await replay(reference, NONDETERMINISM_CHANGED);
  const failed = commandsOf('code-nondeterminism', 'main').activations.find((activation) => activation.failedAttempts)!.failedAttempts![0]!;
  assert.deepEqual(verdict, {
    verdict: 'NONDETERMINISM', activation: 2, commandNumber: failed.report!.commandNumber, actualKind: 'activity', actualTarget: failed.report!.actualTarget,
    actualSagaScope: '', expectedKind: 'activity', expectedTarget: 'email.send', expectedSagaScope: '',
  });
});

test('the second version of code-patch-and-move does not replay the unmatched run, first at command 3', async () => {
  const reference = REFERENCE_RUNS.find((run) => run.scenario === 'code-patch-and-move' && run.run === 'unmatched')!;
  const expectation = (commandsOf('code-patch-and-move', 'unmatched')['expectations'] as { move: { firstDifference: number } }).move;
  const verdict = await replay(reference, PURCHASE_V2);
  assert.equal(verdict.verdict, 'NONDETERMINISM');
  assert.equal(verdict.verdict === 'NONDETERMINISM' && verdict.commandNumber, expectation.firstDifference);
});
