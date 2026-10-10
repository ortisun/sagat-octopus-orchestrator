import { useEffect, useRef, useState } from 'react';
import type { Endpoints } from '../../api/endpoints';
import { isApiError } from '../../api/errors';
import type { DefinitionDiff, Workflow, WorkflowVersion } from '../../api/types';
import { canonical } from '../../model/ir';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { DiffView } from './DiffView';
import type { DraftSession } from './draftSession';
import { Modal } from './Modal';

export const MAX_RELEASE_NOTES = 2_048;

type Stage =
  | { kind: 'preparing'; note?: string }
  | { kind: 'review'; diff: DefinitionDiff; note?: string }
  | { kind: 'publishing'; diff: DefinitionDiff }
  | { kind: 'failed'; error: unknown; what: string; diff?: DefinitionDiff };

interface BodyProps {
  ep: Pick<Endpoints, 'diff' | 'publish' | 'getWorkflow'>;
  workflowId: string;
  session: DraftSession;
  /** The document in the editor, canonical form. */
  currentKey: () => string;
  onClose: () => void;
  onPublished: (version: WorkflowVersion) => void;
  /** The draft moved on under the author: the conflict dialog takes over. */
  onConflict: (theirs: Workflow) => void;
}

function PublishBody({ ep, workflowId, session, currentKey, onClose, onPublished, onConflict }: BodyProps) {
  const [stage, setStage] = useState<Stage>({ kind: 'preparing' });
  const [notes, setNotes] = useState('');
  const [ack, setAck] = useState(false);
  const [round, setRound] = useState(0);
  const attempt = useRef<{ signature: string; key: string } | undefined>(undefined);

  // First the draft is saved, then it is compared with the active version: what is shown is what will be published.
  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      const outcome = await session.save();
      if (controller.signal.aborted) return;
      if (outcome === 'conflict') {
        const theirs = session.getSnapshot().theirs;
        if (theirs) onConflict(theirs);
        return;
      }
      if (outcome === 'failed') {
        setStage({ kind: 'failed', error: session.getSnapshot().error, what: 'Saving the draft' });
        return;
      }
      if (outcome === 'changing') {
        setStage({ kind: 'failed', error: undefined, what: 'Saving the draft, which kept changing,' });
        return;
      }
      try {
        const diff = await ep.diff(workflowId, undefined, undefined, controller.signal);
        if (!controller.signal.aborted) setStage((s) => ({ kind: 'review', diff, ...(s.kind === 'preparing' && s.note ? { note: s.note } : {}) }));
      } catch (error) {
        if (!controller.signal.aborted) setStage({ kind: 'failed', error, what: 'Comparing the draft with the active version' });
      }
    })();
    return () => controller.abort();
  }, [ep, workflowId, session, onConflict, round]);

  const diff = stage.kind === 'review' || stage.kind === 'publishing' ? stage.diff : stage.kind === 'failed' ? stage.diff : undefined;
  const breaking = (diff?.breaking.length ?? 0) > 0;

  const publish = async () => {
    const base = stage.kind === 'review' ? stage.diff : stage.kind === 'failed' ? stage.diff : undefined;
    if (!base) return;
    const revision = session.getSnapshot().revision;
    const trimmed = notes.trim();
    const signature = `${revision}\u0000${trimmed}`;
    const key = attempt.current?.signature === signature ? attempt.current.key : crypto.randomUUID();
    attempt.current = { signature, key };
    setStage({ kind: 'publishing', diff: base });
    try {
      const version = await ep.publish(workflowId, revision, trimmed === '' ? undefined : trimmed, key);
      onPublished(version);
    } catch (error) {
      if (isApiError(error) && error.code === 'CONFLICT') {
        // The draft moved on after it was saved: look at what is there before anything is published.
        try {
          const theirs = await ep.getWorkflow(workflowId);
          if (canonical(theirs.ir) === currentKey()) {
            // The same document under a newer revision (another tab, a repeated save): take the revision and look again.
            await session.overwrite(theirs);
            setAck(false);
            setStage({
              kind: 'preparing',
              note: `The draft was saved again by someone else with the same content (revision ${theirs.draftRevision}). The comparison below is up to date.`,
            });
            setRound((r) => r + 1);
            return;
          }
          onConflict(theirs);
        } catch (inner) {
          setStage({ kind: 'failed', error: inner, what: 'Reading the draft again', diff: base });
        }
        return;
      }
      setStage({ kind: 'failed', error, what: 'Publishing', diff: base });
    }
  };

  const busy = stage.kind === 'preparing' || stage.kind === 'publishing';
  return (
    <div className="wf-publish">
      <p>Publishing creates a new version that never changes. New runs use it; runs already started keep the version they started on.</p>
      {stage.kind === 'preparing' ? <p role="status">{stage.note ? `${stage.note} ` : ''}Saving the draft and comparing it with the active version…</p> : null}
      {stage.kind === 'failed' ? (
        <>
          <ErrorState error={stage.error} what={stage.what} />
          {isApiError(stage.error) && stage.error.code === 'VALIDATION_FAILED' ? (
            <p className="hint">The platform checked the draft again and refused it. The validation panel lists what it found when the editor next checks.</p>
          ) : null}
        </>
      ) : null}
      {stage.kind === 'review' && stage.note ? <Banner tone="info">{stage.note}</Banner> : null}
      {diff ? <DiffView diff={diff} classified /> : null}

      {diff ? (
        <>
          <Field label="Release notes" hint={`Optional. ${notes.length} of ${MAX_RELEASE_NOTES} characters.`}>
            <textarea value={notes} maxLength={MAX_RELEASE_NOTES} rows={3} disabled={stage.kind === 'publishing'} onChange={(event) => setNotes(event.target.value)} />
          </Field>
          {breaking ? (
            <div className="field wf-check">
              <label>
                <input type="checkbox" checked={ack} disabled={stage.kind === 'publishing'} onChange={(event) => setAck(event.target.checked)} /> I understand these changes break
                the contract
              </label>
            </div>
          ) : null}
        </>
      ) : null}

      <div className="dialog-actions">
        <Button onClick={onClose} disabled={stage.kind === 'publishing'}>
          Cancel
        </Button>
        {stage.kind === 'failed' && diff === undefined ? (
          <Button
            onClick={() => {
              setStage({ kind: 'preparing' });
              setRound((r) => r + 1);
            }}
          >
            Try again
          </Button>
        ) : null}
        <Button
          variant="primary"
          onClick={() => void publish()}
          disabled={busy || diff === undefined || (breaking && !ack)}
          aria-describedby={breaking && !ack ? 'wf-publish-ack' : undefined}
        >
          {stage.kind === 'publishing' ? 'Publishing…' : 'Publish'}
        </Button>
      </div>
      {breaking && !ack ? (
        <p id="wf-publish-ack" className="hint">
          Tick the box above to publish: these changes break the contract.
        </p>
      ) : null}
    </div>
  );
}

export function PublishDialog({ open, ...rest }: BodyProps & { open: boolean }) {
  return (
    <Modal open={open} title="Publish workflow" onClose={rest.onClose}>
      <PublishBody {...rest} />
    </Modal>
  );
}
