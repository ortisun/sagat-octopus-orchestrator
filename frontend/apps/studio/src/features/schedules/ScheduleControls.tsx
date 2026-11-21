import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { useNavigate } from 'react-router';
import type { Overlap, Schedule } from '../../api/endpoints';
import { isApiError } from '../../api/errors';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, Dialog, ErrorState, Hint, Link, Row, Select, Stack, Text, TextArea, useAnnounce } from '../../ui';
import { MAX_NOTE, OVERLAPS, OVERLAP_TEXT, noteProblem, outcomeView, reasonText } from './model';
import { useCommand } from './shared';

/** What a person may do to one schedule, each behind the permission the platform will ask for. */
export function ScheduleControls({ schedule, namespaceId }: { schedule: Schedule; namespaceId: string }) {
  const operate = useCan('schedule:operate');
  const write = useCan('schedule:write');
  const start = useCan('run:start');
  const owned = schedule.origin === 'TRIGGER';
  const paused = schedule.state === 'PAUSED';
  // A schedule a cron trigger owns answers to the legacy cron pass until an operator hands the triggers over: the platform says
  // so with `controllable: false`, and refuses the three actions anyway (`SCHEDULE_TRIGGER_HANDOVER_PENDING`).
  const waiting = owned && schedule.controllable === false;
  const reason = 'Not yet: an operator has not handed the cron triggers over to the schedule pass, so the platform refuses this.';
  // Pause when the schedule acts, Resume when it is paused: one button (`Toggle`), so that a pause closes its dialog over the very button
  // that opened it. For a person who may pause and not resume (no `run:start`) the button is gone once the schedule is paused, the dialog
  // with it, and focus would fall to the top of the page: it goes to the heading, which says what the schedule is now.
  const toggle = !(waiting && operate) && (paused ? operate && start : operate && !schedule.exhausted);
  const hadToggle = useRef(toggle);
  useEffect(() => {
    if (hadToggle.current && !toggle && (document.activeElement === null || document.activeElement === document.body)) document.getElementById('schedule-title')?.focus();
    hadToggle.current = toggle;
  }, [toggle]);
  return (
    <Row role="group" aria-label="Actions on the schedule">
      {waiting && operate ? (
        <>
          {!paused && !schedule.exhausted ? (
            <Button disabled title={reason}>
              Pause
            </Button>
          ) : null}
          {start && paused ? (
            <Button disabled title={reason}>
              Resume
            </Button>
          ) : null}
          {start ? (
            <Button disabled title={reason}>
              Run now
            </Button>
          ) : null}
          <Hint role="note">{reason}</Hint>
        </>
      ) : (
        <>
          {toggle ? <Toggle schedule={schedule} namespaceId={namespaceId} /> : null}
          {operate && start ? <Trigger schedule={schedule} namespaceId={namespaceId} /> : null}
        </>
      )}
      {write && start && !owned ? <Link to={`/schedules/${schedule.scheduleId}/edit`}>Edit</Link> : null}
      {write && start && !owned ? <Delete schedule={schedule} namespaceId={namespaceId} /> : null}
    </Row>
  );
}

/** A button that opens a dialog with `children` (given a way to close it) while it is open. */
function Opener({ label, title, variant, children }: { label: string; title: string; variant?: 'primary' | 'danger'; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button {...(variant ? { variant } : {})} onClick={() => setOpen(true)}>
        {label}
      </Button>
      <Dialog open={open} title={title} onClose={() => setOpen(false)}>
        {open ? children(() => setOpen(false)) : null}
      </Dialog>
    </>
  );
}

/**
 * Pause when the schedule acts and Resume when it is paused, as one button: the dialog it opens closes over the button that opened it
 * whatever the schedule has become by then (the page reads it again before the dialog closes), and focus goes back to it (`Dialog`).
 * What the dialog was opened for is kept until it closes, so that its title does not change under the person.
 */
function Toggle({ schedule, namespaceId }: { schedule: Schedule; namespaceId: string }) {
  const paused = schedule.state === 'PAUSED';
  const [opened, setOpened] = useState<'pause' | 'resume' | undefined>(undefined);
  const close = () => setOpened(undefined);
  return (
    <>
      <Button onClick={() => setOpened(paused ? 'resume' : 'pause')}>{paused ? 'Resume' : 'Pause'}</Button>
      <Dialog open={opened !== undefined} title={opened === 'resume' ? 'Resume the schedule' : 'Pause the schedule'} onClose={close}>
        {opened !== undefined ? <NoteForm schedule={schedule} namespaceId={namespaceId} verb={opened} onClose={close} /> : null}
      </Dialog>
    </>
  );
}

function NoteForm({ schedule, namespaceId, verb, onClose }: { schedule: Schedule; namespaceId: string; verb: 'pause' | 'resume'; onClose: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [note, setNote] = useState('');
  const [problem, setProblem] = useState<string>();
  const command = useCommand(`${verb}:${note.trim()}`);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const why = noteProblem(note);
    if (why) {
      setProblem(why);
      return;
    }
    setProblem(undefined);
    const text = note.trim();
    command.send(
      (key, signal) => (verb === 'pause' ? ep.pauseSchedule(namespaceId, schedule.scheduleId, text, key, signal) : ep.resumeSchedule(namespaceId, schedule.scheduleId, text, key, signal)),
      async () => {
        announce(verb === 'pause' ? 'The schedule is paused.' : 'The schedule is resumed.');
        await queries.invalidateQueries({ queryKey: ['schedule', namespaceId, schedule.scheduleId], exact: true });
        await queries.invalidateQueries({ queryKey: ['schedules', namespaceId] });
        onClose();
      },
    );
  };
  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={3}>
        <Text>
          {verb === 'pause'
            ? 'While it is paused no time is acted on, and none is caught up afterwards. Runs already started go on.'
            : 'The times that passed while it was paused are not caught up: it acts again from the next one.'}
        </Text>
        <TextArea label="Why" hint={`1 to ${MAX_NOTE} characters. It is kept with the schedule.`} rows={2} value={note} disabled={command.sending} onChange={setNote} />
        {problem ? <Banner tone="error">{problem}</Banner> : null}
        {command.error ? <ErrorState error={command.error} what={verb === 'pause' ? 'Pausing the schedule' : 'Resuming the schedule'} /> : null}
        <Actions>
          <Button onClick={onClose}>Close</Button>
          <Button type="submit" variant="primary" disabled={command.sending}>
            {command.sending ? 'Saving…' : verb === 'pause' ? 'Confirm pause' : 'Confirm resume'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}

function Trigger({ schedule, namespaceId }: { schedule: Schedule; namespaceId: string }) {
  return (
    <Opener label="Run now" title="Run the schedule now">
      {(close) => <TriggerForm schedule={schedule} namespaceId={namespaceId} onClose={close} />}
    </Opener>
  );
}

function TriggerForm({ schedule, namespaceId, onClose }: { schedule: Schedule; namespaceId: string; onClose: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [overlap, setOverlap] = useState<Overlap | ''>('');
  const [result, setResult] = useState<{ outcome: string; reason: string | null | undefined; runId: string | null | undefined }>();
  const command = useCommand(`trigger:${overlap}`);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    command.send(
      (key, signal) => ep.triggerSchedule(namespaceId, schedule.scheduleId, overlap === '' ? undefined : overlap, key, signal),
      async (action) => {
        setResult({ outcome: action.outcome, reason: action.reason, runId: action.runId });
        announce(`${outcomeView(action.outcome).label}.`);
        await queries.invalidateQueries({ queryKey: ['schedule', namespaceId, schedule.scheduleId], exact: true });
        await queries.invalidateQueries({ queryKey: ['schedule-actions', namespaceId, schedule.scheduleId] });
      },
    );
  };
  if (result) {
    const view = outcomeView(result.outcome);
    return (
      <Stack gap={3}>
        <Banner tone={view.tone === 'bad' ? 'warn' : 'ok'} title={`${view.label}.`}>
          {view.help}{' '}
          {reasonText(result.reason) ? <>{reasonText(result.reason)} </> : null}
          {result.runId ? <Link to={`/runs/${result.runId}`}>Open the run</Link> : null}
        </Banner>
        <Actions>
          <Button onClick={onClose}>Close</Button>
        </Actions>
      </Stack>
    );
  }
  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={3}>
        <Text>
          This decides one time now, as the schedule would at a time of its own. It does not move the next time and, if the schedule has a limit on its actions, it takes one.
          {schedule.state === 'PAUSED' ? ' The schedule is paused; this still runs it once.' : ''}
        </Text>
        <Select<Overlap | ''>
          label="If a run is still open"
          hint={overlap === '' ? `The rule of the schedule applies: ${OVERLAP_TEXT[schedule.policies?.overlap ?? 'SKIP'].label.toLowerCase()}.` : OVERLAP_TEXT[overlap].help}
          value={overlap}
          placeholder="Use the rule of the schedule"
          disabled={command.sending}
          onChange={setOverlap}
          options={[{ value: '', label: 'Use the rule of the schedule' }, ...OVERLAPS.map((name) => ({ value: name, label: OVERLAP_TEXT[name].label }))]}
        />
        {command.error ? <ErrorState error={command.error} what="Running the schedule" /> : null}
        <Actions>
          <Button onClick={onClose}>Close</Button>
          <Button type="submit" variant="primary" disabled={command.sending}>
            {command.sending ? 'Running…' : 'Run now'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}

function Delete({ schedule, namespaceId }: { schedule: Schedule; namespaceId: string }) {
  return (
    <Opener label="Delete" title="Delete the schedule" variant="danger">
      {(close) => <DeleteForm schedule={schedule} namespaceId={namespaceId} onClose={close} />}
    </Opener>
  );
}

function DeleteForm({ schedule, namespaceId, onClose }: { schedule: Schedule; namespaceId: string; onClose: () => void }) {
  // The revision the person opened the dialog at is the one every confirmation asks at, however the page behind it has been read
  // since: after a refusal the page shows the newer schedule, and a confirmation at that revision would delete what they never saw.
  const [revision] = useState(schedule.revision);
  const { ep } = useApp();
  const queries = useQueryClient();
  const navigate = useNavigate();
  const announce = useAnnounce();
  const command = useCommand(`delete:${revision}`);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    command.send(
      (key, signal) => ep.deleteSchedule(namespaceId, schedule.scheduleId, revision, key, signal),
      async () => {
        announce('The schedule was deleted.');
        queries.removeQueries({ queryKey: ['schedule', namespaceId, schedule.scheduleId] });
        await queries.invalidateQueries({ queryKey: ['schedules', namespaceId] });
        void navigate('/schedules');
      },
    );
  };
  const conflict = isApiError(command.error) && command.error.reason === 'REVISION_CONFLICT';
  // The banner tells the person to look at the schedule again: the page behind the dialog reads it again at once, instead of
  // showing the revision the platform just refused until its next round.
  useEffect(() => {
    if (conflict) void queries.invalidateQueries({ queryKey: ['schedule', namespaceId, schedule.scheduleId], exact: true });
  }, [conflict, queries, namespaceId, schedule.scheduleId]);
  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={3}>
        <Text>
          <strong>{schedule.name}</strong> will stop acting. The times that were waiting for a run to end are dropped, runs already started go on, and its history stays. The name becomes free.
        </Text>
        {conflict ? (
          <Banner tone="warn" title="The schedule changed since you opened it.">
            Close this and look at it again before deleting it.
          </Banner>
        ) : null}
        {command.error ? <ErrorState error={command.error} what="Deleting the schedule" /> : null}
        <Actions>
          <Button onClick={onClose}>Close</Button>
          <Button type="submit" variant="danger" disabled={command.sending}>
            {command.sending ? 'Deleting…' : 'Confirm deletion'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}
