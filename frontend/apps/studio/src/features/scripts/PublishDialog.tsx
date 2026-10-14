import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { isApiError } from '../../api/errors';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { StatusBadge } from '../../ui/StatusBadge';
import type { ScriptVersion, ScriptsApi } from './api';
import { CopyButton } from './CopyButton';
import type { DraftSaver } from './draft';
import { startPolling } from './polling';
import type { Poller } from './polling';
import { BUILD_WATCH_MS, keys } from './queries';
import { buildFailureHelp, isVersionFinal, versionStatus } from './status';
import type { Testedness } from './status';
import { testednessText } from './TryPanel';
import { LogView } from './LogView';
import { clipLabel } from './text';

type Phase = 'review' | 'saving' | 'asking' | 'building' | 'ready' | 'failed' | 'expired' | 'error';

/**
 * Publishing a version, in the open. The platform does not require a test run, so a draft nobody
 * tried is a warning here and not a gate; what it does enforce is the build, so a version is only
 * usable (Ready) when the build in isolation succeeded, and the dialog stays until it knows how
 * that ended.
 */
export function PublishDialog({ open, api, scriptId, saver, tested, language, onClose }: { open: boolean; api: ScriptsApi; scriptId: string; saver: DraftSaver; tested: Testedness | undefined; language: string; onClose: () => void }) {
  return (
    <Dialog open={open} title="Publish a version" onClose={onClose}>
      <PublishBody api={api} scriptId={scriptId} saver={saver} tested={tested} language={language} onClose={onClose} />
    </Dialog>
  );
}

function PublishBody({ api, scriptId, saver, tested, language, onClose }: { api: ScriptsApi; scriptId: string; saver: DraftSaver; tested: Testedness | undefined; language: string; onClose: () => void }) {
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [phase, setPhase] = useState<Phase>('review');
  const [message, setMessage] = useState<string | undefined>(undefined);
  const [error, setError] = useState<unknown>(undefined);
  const [version, setVersion] = useState<ScriptVersion | undefined>(undefined);
  const [created, setCreated] = useState(true);
  const poller = useRef<Poller | undefined>(undefined);
  const alive = useRef(true);
  const outcome = useRef<HTMLDivElement>(null);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      poller.current?.stop();
    };
  }, []);
  // When the answer arrives the button that was pressed is gone; the reader is taken to the answer.
  useEffect(() => {
    if (phase === 'ready' || phase === 'failed' || phase === 'expired') outcome.current?.focus();
  }, [phase]);

  const refresh = () => {
    void queries.invalidateQueries({ queryKey: keys.versions(api.namespaceId, scriptId) });
    void queries.invalidateQueries({ queryKey: keys.script(api.namespaceId, scriptId) });
  };

  const finish = (final: ScriptVersion) => {
    setVersion(final);
    setPhase(final.status === 'READY' ? 'ready' : 'failed');
    announce(final.status === 'READY' ? `Version ${final.number} is ready.` : `The build of version ${final.number} failed.`);
    refresh();
  };

  const watch = (first: ScriptVersion) => {
    setVersion(first);
    if (isVersionFinal(first.status)) {
      finish(first);
      return;
    }
    setPhase('building');
    poller.current = startPolling<ScriptVersion>({
      fetch: (signal) => api.version(scriptId, first.number, signal),
      isDone: (current) => isVersionFinal(current.status),
      onValue: (current) => {
        if (alive.current) setVersion(current);
      },
      onSettled: (result) => {
        if (!alive.current) return;
        if (result.kind === 'done') finish(result.value);
        else if (result.kind === 'expired') {
          setPhase('expired');
          announce('The build has not finished. The Studio stopped checking.');
          refresh();
        } else {
          setError(result.error);
          setPhase('error');
        }
      },
      maxDurationMs: BUILD_WATCH_MS,
    });
  };

  const publish = async () => {
    setError(undefined);
    setMessage(undefined);
    setPhase('saving');
    // The platform publishes its copy of the draft: make it the author's.
    const saved = await saver.save();
    if (!alive.current) return;
    if (!saved) {
      setPhase('review');
      setMessage('The draft could not be saved, so nothing was published. See the messages about the draft on the page.');
      return;
    }
    setPhase('asking');
    try {
      const current = await api.get(scriptId);
      if (!alive.current) return;
      if (current.revision !== saver.getSnapshot().revision) {
        // Someone else saved in between: publishing would publish their draft, not this one.
        saver.reportConflict(current.revision);
        setPhase('review');
        setMessage('Someone else changed the draft after you last saved. Nothing was published; decide what to do with the two copies first.');
        return;
      }
      const result = await api.publish(scriptId);
      if (!alive.current) return;
      setCreated(result.created);
      // The list of versions shows the new one (building) at once and follows it by itself.
      refresh();
      watch(result.version);
    } catch (failure) {
      if (!alive.current) return;
      setError(failure);
      setPhase('review');
      if (isApiError(failure) && failure.status === 409) setMessage('The draft changed while it was being published. Nothing new was published.');
    }
  };

  const busy = phase === 'saving' || phase === 'asking';
  return (
    <div className="publish">
      {phase === 'review' || busy ? (
        <>
          <p>
            Publishing freezes the saved draft as the next numbered version and builds it in isolation. The version can be used by a workflow only when the build succeeds; a version never changes after it is published.
          </p>
          {tested?.kind === 'passed' ? <Banner tone="ok">{testednessText(tested)}</Banner> : null}
          {tested !== undefined && tested.kind !== 'passed' ? (
            <Banner tone="warn" title="Not tested.">
              {testednessText(tested)} The platform does not require a test run before publishing, so you can go on; trying it first in the sandbox is better.
            </Banner>
          ) : null}
          {language === 'typescript' ? <p className="hint">TypeScript types are erased when the version is built and are not checked by the platform.</p> : null}
          {message ? <Banner tone="error">{message}</Banner> : null}
          {error !== undefined ? <ErrorState error={error} what="Publishing" /> : null}
          <div className="dialog-actions">
            <Button onClick={onClose}>Cancel</Button>
            <Button variant="primary" onClick={() => void publish()} disabled={busy}>
              {phase === 'saving' ? 'Saving the draft…' : phase === 'asking' ? 'Publishing…' : 'Publish version'}
            </Button>
          </div>
        </>
      ) : (
        <div ref={outcome} tabIndex={-1} className="publish-outcome">
          {version ? (
            <p>
              Version <strong>{version.number}</strong>{' '}
              <StatusBadge view={versionStatus(version.status)} />
              {!created ? ' — the draft is the same as this version, so no new version was made.' : ''}
            </p>
          ) : null}
          {phase === 'building' ? <p role="status">Building in isolation. This usually takes a few seconds; the Studio looks every second or so, and for at most ten minutes.</p> : null}
          {phase === 'ready' && version?.artifactDigest ? (
            <>
              <p>
                <strong>Artifact digest</strong>
              </p>
              <p>
                <code className="digest">{version.artifactDigest}</code>
              </p>
              <p>
                <CopyButton text={version.artifactDigest} label="Copy digest" what={`the digest of version ${version.number}`} />
              </p>
              <p>Use this digest in a Script node of a workflow.</p>
              {version.activity ? (
                <p className="hint">
                  Workers bind it by its activity name, <code>{clipLabel(version.activity, 100)}</code>.
                </p>
              ) : null}
            </>
          ) : null}
          {phase === 'failed' && version ? (
            <>
              <Banner tone="error" title="The build failed.">
                {(version.failureCode || version.failure) ? <code>{clipLabel(version.failureCode || version.failure || '', 300)}</code> : 'The platform gave no reason.'} {buildFailureHelp(version.failureCode ?? version.failure)} Nothing can use this version. {version.failureCode === 'STORAGE_QUOTA_EXCEEDED' ? 'Publish again after capacity is available.' : 'Fix the draft and publish again.'}
              </Banner>
              {version.buildLogTruncated ? <p role="status">Build diagnostics were truncated by the retained-log limit.</p> : null}
              {version.buildLog ? <LogView text={version.buildLog} label="Build log" empty="(empty)" /> : null}
            </>
          ) : null}
          {phase === 'expired' ? (
            <Banner tone="warn" title="The build has not finished.">
              The Studio stopped checking after ten minutes. A sandbox runner may be busy or not running. The version stays in the list with its state; open the Versions tab to see it later.
            </Banner>
          ) : null}
          {phase === 'error' ? <ErrorState error={error} what="Reading the build" /> : null}
          <div className="dialog-actions">
            <Button variant="primary" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
