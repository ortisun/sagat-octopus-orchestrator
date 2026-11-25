import { useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type { IrDocument } from '../../model/ir';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Field } from '../../ui/Field';
import { DurationInput } from './fields';
import type { Change } from './fields';
import { RATE_PERIODS, addPolicy, valueText, isRatePeriod, limitCeiling, limitRangeWords, policyOf, policyProblems, readPolicy, removePolicy, setPolicyField, windowWords } from './flowControl';
import type { Gate, Member, Owner } from './flowControl';

/**
 * The flow-control gates of a workflow (its run starts) or of an `activity` or `script` node (its attempts), feature
 * 022: a concurrency limit and a rate limit, each off until the author turns it on. What it writes is the policy the
 * SDKs write; what it checks is what the platform refuses (decision D022-150).
 */

interface Words {
  title: string;
  limit: string;
  key: string;
  group: string;
  timeout: string;
  add: string;
  remove: string;
  timeoutCode: string;
}

const WORDS: Record<Gate, Words> = {
  concurrency: {
    title: 'Concurrency limit',
    limit: 'Most holders at once',
    key: 'Concurrency key',
    group: 'Concurrency group',
    timeout: 'Concurrency queue timeout',
    add: 'Add a concurrency limit',
    remove: 'Remove the concurrency limit',
    timeoutCode: 'CONCURRENCY_WAIT_TIMEOUT',
  },
  rateLimit: {
    title: 'Rate limit',
    limit: 'Most grants per window',
    key: 'Rate-limit key',
    group: 'Rate-limit group',
    timeout: 'Rate-limit queue timeout',
    add: 'Add a rate limit',
    remove: 'Remove the rate limit',
    timeoutCode: 'RATE_LIMIT_WAIT_TIMEOUT',
  },
};

function ownerKey(owner: Owner): string {
  return owner.level === 'workflow' ? 'workflow' : `node:${owner.nodeId}`;
}

/** A whole number typed as text: what is not one yet is shown and explained, and the document keeps the last good value. */
function LimitInput({
  label,
  hint,
  value,
  max,
  range,
  problem,
  disabled,
  inputRef,
  onCommit,
}: {
  label: string;
  hint: string;
  value: unknown;
  max: number;
  /** The range in words, as the platform's bound says it. */
  range: string;
  problem: string | undefined;
  disabled: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  onCommit: (value: number | undefined) => void;
}) {
  const committed = typeof value === 'number' ? String(value) : '';
  const [typing, setTyping] = useState<{ text: string; base: string; error: string } | undefined>();
  const shown = typing !== undefined && typing.base === committed ? typing : undefined;
  return (
    <Field label={label} hint={hint} error={shown?.error ?? problem}>
      <input
        ref={inputRef}
        type="text"
        inputMode="numeric"
        autoComplete="off"
        aria-required="true"
        value={shown?.text ?? committed}
        disabled={disabled}
        onBlur={() => setTyping(undefined)}
        onChange={(event) => {
          const text = event.target.value.trim();
          if (text === '') {
            setTyping(undefined);
            onCommit(undefined);
            return;
          }
          const number = /^\d{1,10}$/.test(text) ? Number(text) : undefined;
          if (number === undefined || number < 1 || number > max) {
            setTyping({ text, base: committed, error: `Enter a whole number ${range}.` });
            return;
          }
          setTyping(undefined);
          onCommit(number);
        }}
      />
    </Field>
  );
}

function GateFields({ doc, owner, gate, disabled, change }: { doc: IrDocument; owner: Owner; gate: Gate; disabled: boolean; change: Change }) {
  const words = WORDS[gate];
  const view = readPolicy(policyOf(doc, owner, gate), gate);
  const limitRef = useRef<HTMLInputElement | null>(null);
  const [focusLimit, setFocusLimit] = useState(false);
  useEffect(() => {
    if (focusLimit && limitRef.current) {
      limitRef.current.focus();
      setFocusLimit(false);
    }
  }, [focusLimit, view.state]);

  const node = owner.level === 'node';
  const key = `gate:${ownerKey(owner)}:${gate}`;
  const set = (member: Member, value: unknown) => change((d) => setPolicyField(d, owner, gate, member, value), `${key}:${member}`);
  const remove = disabled ? null : (
    <Button aria-label={words.remove} onClick={() => change((d) => removePolicy(d, owner, gate), `${key}:remove`)}>
      Remove
    </Button>
  );

  if (view.state === 'absent') {
    return (
      <fieldset className="field wf-gate">
        <legend>{words.title}</legend>
        <p className="hint">Not limited.</p>
        {disabled ? null : (
          <Button
            onClick={() => {
              change((d) => addPolicy(d, owner, gate), `${key}:add`);
              setFocusLimit(true);
            }}
          >
            {words.add}
          </Button>
        )}
      </fieldset>
    );
  }
  if (view.state === 'unreadable') {
    return (
      <fieldset className="field wf-gate">
        <legend>{words.title}</legend>
        <Banner tone="warn">The {gate} value of this document is not one this Studio can read. It is kept as it is; remove it to declare a new one here.</Banner>
        {remove}
      </fieldset>
    );
  }

  const { policy, unknownMembers } = view;
  const problems = policyProblems(gate, policy);
  const problem = (member: Member) => problems.find((p) => p.member === member)?.text;
  const per = policy['per'];
  const text = (member: Member) => {
    const value = policy[member];
    return typeof value === 'string' ? value : '';
  };
  const timeout = policy['queueTimeoutMs'];
  const timeoutProblem = problem('queueTimeoutMs');
  const admitted = node ? 'the step becomes ready' : 'a run is admitted';

  return (
    <fieldset className="field wf-gate">
      <legend>{words.title}</legend>
      <LimitInput
        label={words.limit}
        hint={`A whole number ${limitRangeWords(gate, per)}. The namespace's ceiling may lower it when ${admitted}.`}
        value={policy['limit']}
        max={limitCeiling(gate, per)}
        range={limitRangeWords(gate, per)}
        problem={problem('limit')}
        disabled={disabled}
        inputRef={limitRef}
        onCommit={(value) => set('limit', value)}
      />
      {gate === 'rateLimit' ? (
        <Field label="Rate-limit window" hint="The length of the sliding window the grants are counted in." error={problem('per')}>
          <select aria-required="true" value={typeof per === 'string' ? per : ''} disabled={disabled} onChange={(event) => set('per', event.target.value === '' ? undefined : event.target.value)}>
            <option value="">(choose a window)</option>
            {per !== undefined && !isRatePeriod(per) ? <option value={valueText(per)}>{valueText(per).slice(0, 20)} (not known to this Studio)</option> : null}
            {RATE_PERIODS.map((p) => (
              <option key={p} value={p}>
                {windowWords(p)}
              </option>
            ))}
          </select>
        </Field>
      ) : null}
      <Field
        label={words.key}
        hint={
          node
            ? 'An expression over input, steps, state or run, read when the step becomes ready. Empty: one key for the whole group.'
            : 'An expression over input, such as input.accountId: runs with the same value share the limit. Empty: one key for the whole group.'
        }
        error={problem('key')}
      >
        <textarea rows={2} spellCheck={false} value={text('key')} disabled={disabled} onChange={(event) => set('key', event.target.value)} />
      </Field>
      <Field
        label={words.group}
        hint={`Workflows of this namespace that name the same group share its keys. Empty: a group of this ${node ? 'step' : 'workflow'} alone.`}
        error={problem('group')}
      >
        <input type="text" autoComplete="off" spellCheck={false} value={text('group')} disabled={disabled} onChange={(event) => set('group', event.target.value)} />
      </Field>
      <DurationInput
        label={words.timeout}
        hint={`How long ${node ? 'the step' : 'a run'} waits for room before it fails with ${words.timeoutCode}. Empty: ten minutes; at most one day.`}
        value={typeof timeout === 'number' ? timeout : undefined}
        disabled={disabled}
        onCommit={(ms) => set('queueTimeoutMs', ms)}
      />
      {timeoutProblem ? (
        <span className="error" role="alert">
          {timeoutProblem}
        </span>
      ) : null}
      {unknownMembers.length > 0 ? (
        <p className="hint">
          It also holds {unknownMembers.map((m) => m.slice(0, 40)).join(', ')}, which this Studio does not know: kept as it is.
        </p>
      ) : null}
      {remove}
    </fieldset>
  );
}

export function FlowControlFields({ doc, owner, disabled, change }: { doc: IrDocument; owner: Owner; disabled: boolean; change: Change }) {
  const node = owner.level === 'node';
  return (
    <section aria-label="Flow control" className="wf-flow-control">
      <h3>Flow control</h3>
      <p className="hint">
        {node
          ? 'Limits on the attempts of this step: how many of one key run at once, and how many are dispatched in a window. A step over a limit waits without spending an attempt.'
          : 'Limits on the runs of this workflow: how many of one key run at once, and how many start in a window. A run over a limit waits in a queue, in the order it arrived.'}{' '}
        Publishing a limit needs the platform&apos;s capability for it switched on and activated; the checks say when it is not.
      </p>
      <GateFields doc={doc} owner={owner} gate="concurrency" disabled={disabled} change={change} />
      <GateFields doc={doc} owner={owner} gate="rateLimit" disabled={disabled} change={change} />
    </section>
  );
}
