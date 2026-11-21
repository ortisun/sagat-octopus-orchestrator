import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { useNavigate, useParams } from 'react-router';
import { isApiError } from '../../api/errors';
import type { Overlap } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, Checkbox, Code, ErrorState, Link, Loading, Page, PageHeader, Row, Select, Stack, Text, TextArea, TextInput, useAnnounce } from '../../ui';
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
  if (namespace.pending || current.isPending) return <Loading>Loading the schedule…</Loading>;
  if (namespace.error || current.error) return <ErrorState error={namespace.error ?? current.error} what="Loading the schedule" onRetry={() => void current.refetch()} />;
  const schedule = current.data;
  if (schedule.origin === 'TRIGGER') {
    return (
      <Page>
        <Stack as="section" gap={4}>
          <PageHeader title={`Edit ${schedule.name}`} />
          <Banner tone="info">A cron trigger owns this schedule, so it is changed through the trigger.</Banner>
          <Text>
            <Link to={`/schedules/${id}`}>Back to the schedule</Link>
          </Text>
        </Stack>
      </Page>
    );
  }
  const editable = draftOf(schedule);
  if ('cannot' in editable) {
    return (
      <Page>
        <Stack as="section" gap={4}>
          <PageHeader title={`Edit ${schedule.name}`} />
          <Banner tone="info">{editable.cannot}</Banner>
          <Text>
            <Link to={`/schedules/${id}`}>Back to the schedule</Link>
          </Text>
        </Stack>
      </Page>
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
    <Page>
      <Stack as="section" gap={4} aria-labelledby="schedule-form-title">
        <Text>
          <Link to={existing ? `/schedules/${existing.id}` : '/schedules'}>{existing ? 'Back to the schedule' : 'All schedules'}</Link>
        </Text>
        <PageHeader title={<span id="schedule-form-title">{existing ? `Edit ${existing.name}` : 'New schedule'}</span>} />
        <form onSubmit={submit} noValidate>
          <Stack gap={3}>
            <TextInput label="Name" hint="1 to 128 characters; unique among the schedules of the namespace." autoComplete="off" value={draft.name} onChange={(value) => set('name', value)} />
            <Select
              label="Workflow"
              hint={pinned ? 'The schedule is pinned to one version, which this form keeps.' : 'The runs start its active version. Only workflows with a published version are offered.'}
              value={draft.workflowId}
              placeholder="Choose a workflow"
              onChange={(value) => set('workflowId', value)}
              options={[
                ...(draft.workflowId !== '' && !known ? [{ value: draft.workflowId, label: shortId(draft.workflowId) }] : []),
                ...choices.map((workflow) => ({ value: workflow.id, label: workflow.name })),
              ]}
            />
            <TextArea
              label="Input"
              hint={existing ? 'A JSON object, at most 64 KiB. The platform never returns the input of a schedule, so write it again: what you save replaces it.' : 'A JSON object, at most 64 KiB, given to every run.'}
              rows={5}
              spellCheck={false}
              mono
              value={draft.input}
              onChange={(value) => set('input', value)}
            />

            <TextInput
              label="Business id prefix"
              hint={`Optional: every run the schedule starts is given the business id "<prefix>-<time>". 1 to ${MAX_BUSINESS_PREFIX} letters, digits and punctuation. ${[...prefix].length} of ${MAX_BUSINESS_PREFIX} characters.`}
              error={prefixProblem ?? (prefixRefused ? `The platform refused this prefix: it takes 1 to ${MAX_BUSINESS_PREFIX} letters, digits and punctuation, with no space, accent or control character.` : undefined)}
              autoComplete="off"
              spellCheck={false}
              mono
              value={draft.businessIdPrefix}
              onChange={(value) => set('businessIdPrefix', value)}
            />
            {prefix !== '' ? (
              <>
                <Row gap={3} align="start">
                  <Select<BusinessConflict>
                    label="If an open run already has the id"
                    width={320}
                    value={draft.businessIdConflictPolicy}
                    onChange={(value) => set('businessIdConflictPolicy', value)}
                    options={BUSINESS_CONFLICTS.map((value) => ({ value, label: `${BUSINESS_CONFLICT_TEXT[value].label}${value === DEFAULT_CONFLICT ? ' (default)' : ''}` }))}
                  />
                  <Select<BusinessReuse>
                    label="If a closed run still remembers the id"
                    width={320}
                    value={draft.businessIdReusePolicy}
                    onChange={(value) => set('businessIdReusePolicy', value)}
                    options={BUSINESS_REUSES.map((value) => ({ value, label: `${BUSINESS_REUSE_TEXT[value].label}${value === DEFAULT_REUSE ? ' (default)' : ''}` }))}
                  />
                </Row>
                {prefixProblem === undefined ? (
                  <div style={{ overflowWrap: 'anywhere' }}>
                    <Text>
                      {nextNominalTime === undefined ? (
                        <>
                          Each run gets the prefix, a dash and the nominal time of its action in UTC, for example <Code>{businessIdPreview(prefix, undefined, openedAt)}</Code>.
                        </>
                      ) : (
                        <>
                          The next action would start its run under the business id <Code>{businessIdPreview(prefix, nextNominalTime, openedAt)}</Code>.
                        </>
                      )}
                    </Text>
                  </div>
                ) : null}
                <Text>{businessPolicySentence(draft.businessIdConflictPolicy, draft.businessIdReusePolicy)}</Text>
              </>
            ) : null}

            <Select<When>
              label="Repeat by"
              value={draft.when}
              onChange={(value) => set('when', value)}
              options={[
                { value: 'INTERVAL', label: 'An interval' },
                { value: 'CRON', label: 'Cron expressions' },
              ]}
            />
            {draft.when === 'INTERVAL' ? (
              <Row gap={3} align="start">
                <TextInput label="Every" width={160} numeric autoComplete="off" value={draft.every} onChange={(value) => set('every', value)} />
                <Select<Unit> label="Unit" width={200} value={draft.unit} onChange={(value) => set('unit', value)} options={(Object.keys(UNITS) as Unit[]).map((unit) => ({ value: unit, label: UNIT_LABEL[unit] }))} />
              </Row>
            ) : (
              <TextArea
                label="Cron expressions"
                hint="Five fields (minute, hour, day of month, month, day of week), one expression to a line, up to ten."
                rows={3}
                spellCheck={false}
                mono
                value={draft.cron}
                onChange={(value) => set('cron', value)}
              />
            )}
            <TextInput label="Time zone" hint="An IANA name, such as UTC or Europe/Lisbon. Cron expressions are read in it; intervals are not." autoComplete="off" value={draft.timezone} onChange={(value) => set('timezone', value)} />
            <TextInput
              label="Jitter (ms)"
              hint="A steady offset of up to this many milliseconds for each time, so many schedules do not start together. 0 for none."
              numeric
              autoComplete="off"
              value={draft.jitterMs}
              onChange={(value) => set('jitterMs', value)}
            />
            <Row gap={3} align="start">
              <TextInput label="Not before" width={280} hint="Optional, such as 2026-12-01T09:00:00Z." autoComplete="off" spellCheck={false} mono value={draft.startAt} onChange={(value) => set('startAt', value)} />
              <TextInput label="Not at or after" width={280} hint="Optional; the end is exclusive." autoComplete="off" spellCheck={false} mono value={draft.endAt} onChange={(value) => set('endAt', value)} />
            </Row>

            <Select<Overlap>
              label="When a run is still open"
              hint={OVERLAP_TEXT[draft.overlap].help}
              value={draft.overlap}
              onChange={(value) => set('overlap', value)}
              options={OVERLAPS.map((name) => ({ value: name, label: OVERLAP_TEXT[name].label }))}
            />
            <Select<Draft['catchupMode']>
              label="Catching up"
              hint="After an outage: start only the latest missed time, or every one of them."
              value={draft.catchupMode}
              onChange={(value) => set('catchupMode', value)}
              options={[
                { value: 'ONE', label: 'Only the latest missed time' },
                { value: 'ALL', label: 'Every missed time' },
              ]}
            />
            <Checkbox label="Pause the schedule when a run it started fails" checked={draft.pauseOnFailure} onChange={(checked) => set('pauseOnFailure', checked)} />
            <TextInput
              label="Actions left"
              hint="Optional: stop after this many runs have been started. Empty for no limit."
              numeric
              autoComplete="off"
              value={draft.remainingActions}
              onChange={(value) => set('remainingActions', value)}
            />
            <Checkbox label={existing ? 'Keep the schedule paused' : 'Create the schedule paused'} checked={draft.paused} onChange={(checked) => set('paused', checked)} />
            <TextArea label="Note" hint={`Optional, up to ${MAX_NOTE} characters: why it exists or why it is paused.`} rows={2} value={draft.note} onChange={(value) => set('note', value)} />

            <Text>You become the person the schedule acts for: every run it starts is checked against your permission to start runs.</Text>
            {problem ? <Banner tone="error">{problem}</Banner> : null}
            {conflict ? (
              <Banner tone="warn" title="The schedule changed while you were editing.">
                Someone saved a newer version. Go back to the schedule to see it, then edit it again.
              </Banner>
            ) : null}
            {command.error ? <ErrorState error={command.error} what={existing ? 'Saving the schedule' : 'Creating the schedule'} /> : null}
            <Actions>
              <Button type="submit" variant="primary" disabled={command.sending || namespace.id === undefined}>
                {command.sending ? 'Saving…' : existing ? 'Save changes' : 'Create schedule'}
              </Button>
            </Actions>
          </Stack>
        </form>
      </Stack>
    </Page>
  );
}
