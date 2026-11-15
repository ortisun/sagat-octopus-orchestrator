import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { isApiError } from '../../api/errors';
import type { Overlap } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { NotAllowed } from '../runs/NotAllowed';
import { useWorkflowIndex } from '../runs/queries';
import {
  BUSINESS_CONFLICTS,
  BUSINESS_CONFLICT_TEXT,
  BUSINESS_REUSES,
  BUSINESS_REUSE_TEXT,
  DEFAULT_CONFLICT,
  DEFAULT_REUSE,
  EMPTY_DRAFT,
  MAX_BUSINESS_PREFIX,
  MAX_NOTE,
  OVERLAPS,
  OVERLAP_TEXT,
  UNITS,
  UNIT_LABEL,
  businessIdPreview,
  businessPolicySentence,
  businessPrefixProblem,
  draftOf,
  refusedPrefix,
  requestOf,
} from './model';
import type { BusinessConflict, BusinessReuse, Draft, Unit, When } from './model';
import { useCommand, useNamespaceId } from './shared';
import { shortId } from '../runs/display';

/** `/schedules/new` and `/schedules/:scheduleId/edit`: the same form, empty or filled with the schedule. */
export function ScheduleFormPage() {
  const { scheduleId } = useParams();
  const canWrite = useCan('schedule:write');
  const canStart = useCan('run:start');
  if (!canWrite) return <NotAllowed what="Schedule editing" permission="schedule:write" />;
  if (!canStart) return <NotAllowed what="Schedule editing" permission="run:start" />;
  // A different schedule is a different form: nothing typed for the last one carries over.
  return scheduleId ? <Edit key={scheduleId} id={scheduleId} /> : <Editor key="new" initial={EMPTY_DRAFT} />;
}

function Edit({ id }: { id: string }) {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const current = useQuery({
    queryKey: ['schedule', namespace.id, id, 'form'],
    enabled: namespace.id !== undefined,
    // The form is the person's copy of one revision: never refetched under them while they type
    // (the revision they save against must be the one they started from), and never kept after.
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
    // One next time, for the preview of the business id: the platform calculates it, the form calculates none.
    queryFn: ({ signal }) => ep.getSchedule(namespace.id as string, id, { nextCount: 1, recentCount: 0 }, signal),
  });
  if (namespace.pending || current.isPending) return <p role="status">Loading the schedule…</p>;
  if (namespace.error || current.error) return <ErrorState error={namespace.error ?? current.error} what="Loading the schedule" onRetry={() => void current.refetch()} />;
  const schedule = current.data;
  if (schedule.origin === 'TRIGGER') {
    return (
      <section>
        <h1>Edit {schedule.name}</h1>
        <Banner tone="info">A cron trigger owns this schedule, so it is changed through the trigger.</Banner>
        <p>
          <Link to={`/schedules/${id}`}>Back to the schedule</Link>
        </p>
      </section>
    );
  }
  const editable = draftOf(schedule);
  if ('cannot' in editable) {
    return (
      <section>
        <h1>Edit {schedule.name}</h1>
        <Banner tone="info">{editable.cannot}</Banner>
        <p>
          <Link to={`/schedules/${id}`}>Back to the schedule</Link>
        </p>
      </section>
    );
  }
  return <Editor initial={editable.draft} existing={{ id, revision: schedule.revision, name: schedule.name }} nextNominalTime={schedule.nextActions[0]?.nominalTime} />;
}

function Editor({ initial, existing: opened, nextNominalTime }: { initial: Draft; existing?: { id: string; revision: number; name: string }; nextNominalTime?: string | undefined }) {
  // The revision the person opened the schedule at is the one every save asks at: a copy that were read again under them (by a refresh
  // of the query it came from) would turn a refusal into an acceptance of somebody else's change.
  const [existing] = useState(opened);
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const queries = useQueryClient();
  const navigate = useNavigate();
  const announce = useAnnounce();
  const workflows = useWorkflowIndex();
  const [draft, setDraft] = useState<Draft>(initial);
  const [problem, setProblem] = useState<string>();
  // The clock of the example id is read once: it does not move under the person while they type.
  const [openedAt] = useState(() => new Date());
  // The prefix the last request carried: a refusal of the platform is about that text, so it goes when the text changes.
  const [sentPrefix, setSentPrefix] = useState<string>();
  const request = requestOf(draft);
  const command = useCommand(JSON.stringify([existing?.id ?? null, existing?.revision ?? null, 'body' in request ? request.body : null]));
  const set = <K extends keyof Draft>(field: K, value: Draft[K]) => setDraft((last) => ({ ...last, [field]: value }));

  const choices = (workflows.data?.list ?? []).filter((workflow) => workflow.activeVersion || workflow.id === draft.workflowId);
  const known = choices.some((workflow) => workflow.id === draft.workflowId);
  const pinned = draft.carry.versionPolicy === 'PINNED';
  const prefix = draft.businessIdPrefix.trim();
  const prefixProblem = businessPrefixProblem(draft.businessIdPrefix);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (command.sending || namespace.id === undefined) return;
    if ('problem' in request) {
      setProblem(request.problem);
      return;
    }
    setProblem(undefined);
    setSentPrefix(prefix);
    const namespaceId = namespace.id;
    const body = request.body;
    command.send(
      (key, signal) =>
        existing
          ? ep.updateSchedule(namespaceId, existing.id, { ...body, expectedRevision: existing.revision }, key, signal)
          : ep.createSchedule(namespaceId, body, key, signal),
      async (saved) => {
        announce(existing ? 'The schedule was saved.' : 'The schedule was created.');
        await queries.invalidateQueries({ queryKey: ['schedules', namespaceId] });
        await queries.invalidateQueries({ queryKey: ['schedule', namespaceId, saved.scheduleId], exact: true });
        void navigate(`/schedules/${saved.scheduleId}`);
      },
    );
  };

  const conflict = isApiError(command.error) && command.error.reason === 'REVISION_CONFLICT';
  const prefixRefused = refusedPrefix(command.error) && sentPrefix === prefix;
  // The banner sends the person back to the schedule to see what the other person saved: the page they came from may have read it
  // moments ago and would still show the old one, so what it holds is marked out of date and read again when it is shown. Only that
  // key, and only marked (`exact`, `refetchType: 'none'`): the key of this form (`[..., 'form']`, a prefix match would reach it) is a
  // copy that is never read again under the person, so that saving again is refused again until they have looked.
  const scheduleId = existing?.id;
  const namespaceId = namespace.id;
  useEffect(() => {
    if (conflict && scheduleId !== undefined && namespaceId !== undefined) {
      void queries.invalidateQueries({ queryKey: ['schedule', namespaceId, scheduleId], exact: true, refetchType: 'none' });
    }
  }, [conflict, scheduleId, namespaceId, queries]);
  return (
    <section aria-labelledby="schedule-form-title">
      <p>
        <Link to={existing ? `/schedules/${existing.id}` : '/schedules'}>{existing ? 'Back to the schedule' : 'All schedules'}</Link>
      </p>
      <h1 id="schedule-form-title">{existing ? `Edit ${existing.name}` : 'New schedule'}</h1>
      <form onSubmit={submit} noValidate>
        <Field label="Name" hint="1 to 128 characters; unique among the schedules of the namespace.">
          <input type="text" autoComplete="off" value={draft.name} onChange={(event) => set('name', event.target.value)} />
        </Field>
        <Field label="Workflow" hint={pinned ? 'The schedule is pinned to one version, which this form keeps.' : 'The runs start its active version. Only workflows with a published version are offered.'}>
          <select value={draft.workflowId} onChange={(event) => set('workflowId', event.target.value)}>
            <option value="">Choose a workflow</option>
            {draft.workflowId !== '' && !known ? <option value={draft.workflowId}>{shortId(draft.workflowId)}</option> : null}
            {choices.map((workflow) => (
              <option key={workflow.id} value={workflow.id}>
                {workflow.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Input" hint={existing ? 'A JSON object, at most 64 KiB. The platform never returns the input of a schedule, so write it again: what you save replaces it.' : 'A JSON object, at most 64 KiB, given to every run.'}>
          <textarea rows={5} spellCheck={false} autoComplete="off" value={draft.input} onChange={(event) => set('input', event.target.value)} />
        </Field>

        <Field
          label="Business id prefix"
          hint={`Optional: every run the schedule starts is given the business id "<prefix>-<time>". 1 to ${MAX_BUSINESS_PREFIX} letters, digits and punctuation. ${[...prefix].length} of ${MAX_BUSINESS_PREFIX} characters.`}
          error={prefixProblem ?? (prefixRefused ? `The platform refused this prefix: it takes 1 to ${MAX_BUSINESS_PREFIX} letters, digits and punctuation, with no space, accent or control character.` : undefined)}
        >
          <input type="text" autoComplete="off" spellCheck={false} value={draft.businessIdPrefix} onChange={(event) => set('businessIdPrefix', event.target.value)} />
        </Field>
        {prefix !== '' ? (
          <>
            <div className="toolbar">
              <Field label="If an open run already has the id">
                <select value={draft.businessIdConflictPolicy} onChange={(event) => set('businessIdConflictPolicy', event.target.value as BusinessConflict)}>
                  {BUSINESS_CONFLICTS.map((value) => (
                    <option key={value} value={value}>
                      {BUSINESS_CONFLICT_TEXT[value].label}
                      {value === DEFAULT_CONFLICT ? ' (default)' : ''}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="If a closed run still remembers the id">
                <select value={draft.businessIdReusePolicy} onChange={(event) => set('businessIdReusePolicy', event.target.value as BusinessReuse)}>
                  {BUSINESS_REUSES.map((value) => (
                    <option key={value} value={value}>
                      {BUSINESS_REUSE_TEXT[value].label}
                      {value === DEFAULT_REUSE ? ' (default)' : ''}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            {prefixProblem === undefined ? (
              <p style={{ overflowWrap: 'anywhere' }}>
                {nextNominalTime === undefined ? (
                  <>
                    Each run gets the prefix, a dash and the nominal time of its action in UTC, for example <code>{businessIdPreview(prefix, undefined, openedAt)}</code>.
                  </>
                ) : (
                  <>
                    The next action would start its run under the business id <code>{businessIdPreview(prefix, nextNominalTime, openedAt)}</code>.
                  </>
                )}
              </p>
            ) : null}
            <p>{businessPolicySentence(draft.businessIdConflictPolicy, draft.businessIdReusePolicy)}</p>
          </>
        ) : null}

        <Field label="Repeat by">
          <select value={draft.when} onChange={(event) => set('when', event.target.value as When)}>
            <option value="INTERVAL">An interval</option>
            <option value="CRON">Cron expressions</option>
          </select>
        </Field>
        {draft.when === 'INTERVAL' ? (
          <div className="toolbar">
            <Field label="Every">
              <input type="text" inputMode="numeric" autoComplete="off" value={draft.every} onChange={(event) => set('every', event.target.value)} />
            </Field>
            <Field label="Unit">
              <select value={draft.unit} onChange={(event) => set('unit', event.target.value as Unit)}>
                {(Object.keys(UNITS) as Unit[]).map((unit) => (
                  <option key={unit} value={unit}>
                    {UNIT_LABEL[unit]}
                  </option>
                ))}
              </select>
            </Field>
          </div>
        ) : (
          <Field label="Cron expressions" hint="Five fields (minute, hour, day of month, month, day of week), one expression to a line, up to ten.">
            <textarea rows={3} spellCheck={false} autoComplete="off" value={draft.cron} onChange={(event) => set('cron', event.target.value)} />
          </Field>
        )}
        <Field label="Time zone" hint="An IANA name, such as UTC or Europe/Lisbon. Cron expressions are read in it; intervals are not.">
          <input type="text" autoComplete="off" value={draft.timezone} onChange={(event) => set('timezone', event.target.value)} />
        </Field>
        <Field label="Jitter (ms)" hint="A steady offset of up to this many milliseconds for each time, so many schedules do not start together. 0 for none.">
          <input type="text" inputMode="numeric" autoComplete="off" value={draft.jitterMs} onChange={(event) => set('jitterMs', event.target.value)} />
        </Field>
        <div className="toolbar">
          <Field label="Not before" hint="Optional, such as 2026-12-01T09:00:00Z.">
            <input type="text" autoComplete="off" value={draft.startAt} onChange={(event) => set('startAt', event.target.value)} />
          </Field>
          <Field label="Not at or after" hint="Optional; the end is exclusive.">
            <input type="text" autoComplete="off" value={draft.endAt} onChange={(event) => set('endAt', event.target.value)} />
          </Field>
        </div>

        <Field label="When a run is still open" hint={OVERLAP_TEXT[draft.overlap].help}>
          <select value={draft.overlap} onChange={(event) => set('overlap', event.target.value as Overlap)}>
            {OVERLAPS.map((name) => (
              <option key={name} value={name}>
                {OVERLAP_TEXT[name].label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Catching up" hint="After an outage: start only the latest missed time, or every one of them.">
          <select value={draft.catchupMode} onChange={(event) => set('catchupMode', event.target.value as Draft['catchupMode'])}>
            <option value="ONE">Only the latest missed time</option>
            <option value="ALL">Every missed time</option>
          </select>
        </Field>
        <p>
          <label>
          <input type="checkbox" checked={draft.pauseOnFailure} onChange={(event) => set('pauseOnFailure', event.target.checked)} /> Pause the schedule when a run it started fails
          </label>
        </p>
        <Field label="Actions left" hint="Optional: stop after this many runs have been started. Empty for no limit.">
          <input type="text" inputMode="numeric" autoComplete="off" value={draft.remainingActions} onChange={(event) => set('remainingActions', event.target.value)} />
        </Field>
        <p>
          <label>
          <input type="checkbox" checked={draft.paused} onChange={(event) => set('paused', event.target.checked)} /> {existing ? 'Keep the schedule paused' : 'Create the schedule paused'}
          </label>
        </p>
        <Field label="Note" hint={`Optional, up to ${MAX_NOTE} characters: why it exists or why it is paused.`}>
          <textarea rows={2} autoComplete="off" value={draft.note} onChange={(event) => set('note', event.target.value)} />
        </Field>

        <p>You become the person the schedule acts for: every run it starts is checked against your permission to start runs.</p>
        {problem ? (
          <p role="alert" className="error">
            {problem}
          </p>
        ) : null}
        {conflict ? (
          <Banner tone="warn" title="The schedule changed while you were editing.">
            Someone saved a newer version. Go back to the schedule to see it, then edit it again.
          </Banner>
        ) : null}
        {command.error ? <ErrorState error={command.error} what={existing ? 'Saving the schedule' : 'Creating the schedule'} /> : null}
        <div className="toolbar">
          <Button type="submit" variant="primary" disabled={command.sending || namespace.id === undefined}>
            {command.sending ? 'Saving…' : existing ? 'Save changes' : 'Create schedule'}
          </Button>
        </div>
      </form>
    </section>
  );
}
