import { useId } from 'react';
import type { ReactNode } from 'react';
import type { Run } from '../../api/types';
import { gateWords, rateWords } from './gates';
import type { GateView } from './gates';
import { Moment } from './Now';
import './gates.css';

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function Gate({ view }: { view: GateView }) {
  const id = useId();
  return (
    <div role="group" aria-labelledby={id} className="rn-gate" data-waiting={view.waiting ? 'true' : undefined}>
      <h3 id={id}>{view.heading}</h3>
      <dl className="rn-facts rn-grid">
        <Fact label="State">{view.state}</Fact>
        {view.since ? (
          <Fact label={view.sinceLabel}>
            <Moment at={view.since} />
          </Fact>
        ) : null}
        {view.waiting && view.deadline ? (
          <Fact label="Gives up at">
            <Moment at={view.deadline} /> (fails with {view.deadlineCode} if it is not granted before)
          </Fact>
        ) : null}
        {view.limit ? <Fact label="Limit">{view.limit}</Fact> : null}
        {view.position ? <Fact label="Place">{view.position}</Fact> : null}
        {view.estimate !== undefined ? (
          <Fact label="Estimated grant">
            {view.estimate === null ? (
              view.estimateWithheld ? (
                'Withheld: you may not reveal personal data in this namespace, so the platform does not give an estimate.'
              ) : (
                'None at this read: the platform gives an estimate only when it can compute one.'
              )
            ) : (
              <>
                <Moment at={view.estimate} /> (an estimate, not a promise)
              </>
            )}
          </Fact>
        ) : null}
        {view.countsUntil ? (
          <Fact label="Counts until">
            <Moment at={view.countsUntil} />
          </Fact>
        ) : null}
        <Fact label="Group">{view.group}</Fact>
        <Fact label="Pool">{view.pool}</Fact>
        <Fact label="Key label">{view.key}</Fact>
        {view.unsupported.length > 0 ? <Fact label="Unsupported">{view.unsupported.join(', ')}: shown as the platform sent it</Fact> : null}
      </dl>
    </div>
  );
}

/**
 * The run-level gates of feature 022 that the single-run read returns: the concurrency permit the run holds or waits
 * for and the rate-limit slot it was granted or waits for (FR-022-022, FR-022-048). Nothing for a run without either.
 */
export function RunGates({ run }: { run: Run }) {
  const views = [gateWords(run.concurrency), rateWords(run.rateLimit)].filter((view): view is GateView => view !== undefined);
  if (views.length === 0) return null;
  return (
    <section className="rn-gates" aria-label="Flow control">
      <h2 className="rn-micro-label">Flow control</h2>
      {views.map((view) => (
        <Gate key={view.heading} view={view} />
      ))}
    </section>
  );
}
