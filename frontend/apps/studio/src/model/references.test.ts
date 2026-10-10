import { describe, expect, it } from 'vitest';
import { danglingReferences, renameStepReference, stepReferences } from './references';

describe('the reads of another step in an expression', () => {
  it('finds the dot form and the index form, and what they read', () => {
    expect(stepReferences('steps.charge.output.ok == true && steps["pay-2"].output.id != ""').map((r) => [r.id, r.form])).toEqual([
      ['charge', 'dot'],
      ['pay-2', 'index'],
    ]);
    expect(stepReferences("steps ['a.b'] .output").map((r) => r.id)).toEqual(['a.b']);
    expect(stepReferences('steps . charge').map((r) => r.id)).toEqual(['charge']);
  });

  it('does not find what is not a read: a string, a member called steps, a longer name, plain input', () => {
    expect(stepReferences('input.steps.charge == "steps.charge.output"')).toEqual([]);
    expect(stepReferences("'steps.charge' + mysteps.charge + steps_x.charge")).toEqual([]);
    expect(stepReferences('input.amount > 1000')).toEqual([]);
    expect(stepReferences('steps.')).toEqual([]);
  });

  it('reads a template only inside ${…}', () => {
    expect(stepReferences('steps.a.output:${steps.b.output.id}:${run.id}', true).map((r) => r.id)).toEqual(['b']);
    expect(stepReferences('${"}" + steps.c.output}', true).map((r) => r.id)).toEqual(['c']);
  });
});

describe('renaming a step in the expressions that read it', () => {
  it('renames every read of that step and no other', () => {
    expect(renameStepReference('steps.charge.output.ok && steps.charge.output.id != steps.other.output.id', 'charge', 'payment')).toBe(
      'steps.payment.output.ok && steps.payment.output.id != steps.other.output.id',
    );
    expect(renameStepReference('steps["charge"].output', 'charge', 'payment')).toBe('steps["payment"].output');
    expect(renameStepReference("steps['charge'].output", 'charge', 'pay-2')).toBe("steps['pay-2'].output");
  });

  it('turns a dot read into an index read when the new id is not a name', () => {
    expect(renameStepReference('steps.charge.output', 'charge', 'charge-card')).toBe('steps["charge-card"].output');
    expect(renameStepReference('steps . charge . output', 'charge', 'a.b')).toBe('steps ["a.b"] . output');
  });

  it('leaves strings and members alone, and renames inside a template', () => {
    expect(renameStepReference('"steps.charge" == x.steps.charge', 'charge', 'p')).toBe('"steps.charge" == x.steps.charge');
    expect(renameStepReference('pay:${steps.charge.output.id}:steps.charge', 'charge', 'p', true)).toBe('pay:${steps.p.output.id}:steps.charge');
  });
});

describe('reads of a step that is not in the document', () => {
  it('names the node, the field and the missing step', () => {
    const nodes = [
      { id: 'a', type: 'activity', activity: 'x.y', idempotencyTemplate: 'k:${steps.gone.output.id}' },
      { id: 'c', type: 'condition', expression: 'steps.a.output.ok && steps.removed.output.ok', ifTrue: 'a', ifFalse: 'a' },
      { id: 'e', type: 'end' },
    ];
    expect(danglingReferences(nodes)).toEqual([
      { nodeId: 'a', field: 'idempotencyTemplate', id: 'gone' },
      { nodeId: 'c', field: 'expression', id: 'removed' },
    ]);
  });
});
