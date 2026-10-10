import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Link, useParams } from 'react-router';
import { useCan } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Tabs } from '../../ui/Tabs';
import { useAnnounce } from '../../ui/Announcer';
import '../../styles/scripts.css';
import type { Script, ScriptsApi } from './api';
import { CodeEditor } from './CodeEditor';
import type { EditorControls } from './CodeEditor';
import { scriptsConfig } from './config';
import { ConflictPanel } from './ConflictPanel';
import { DraftSaver } from './draft';
import type { DraftSnapshot } from './draft';
import { schemaToText } from './jsonText';
import { LIMITS, SAMPLE_INPUT, languageInfo, monacoLanguage } from './languages';
import { LimitsPanel } from './LimitsPanel';
import type { Problem } from './monacoSetup';
import { ProblemsList } from './ProblemsList';
import { PublishDialog } from './PublishDialog';
import { keys, useVersionList } from './queries';
import { SchemasPanel } from './SchemasPanel';
import { testedness } from './status';
import { clipLabel, formatBytes, isUuid, utf8Length } from './text';
import { TryPanel } from './TryPanel';
import { useScriptsApi } from './useScriptsApi';
import { useTestRun } from './useTestRun';
import { VersionView } from './VersionView';
import { VersionsPanel } from './VersionsPanel';
import { isApiError } from '../../api/errors';

/** One script: its draft in an editor, the sandbox to try it in, its schemas, and its published versions. */
export function ScriptEditorPage() {
  const { scriptId } = useParams();
  const context = useScriptsApi();
  const canRead = useCan('script:read');
  return (
    <section className="script-editor" aria-labelledby="script-title">
      {!canRead ? (
        <>
          <h1 id="script-title">Script</h1>
          <Banner tone="warn" title="Your role cannot see scripts.">
            Reading scripts needs the permission <code>script:read</code>.
          </Banner>
        </>
      ) : !isUuid(scriptId) ? (
        <>
          <h1 id="script-title">Script not found</h1>
          <p>
            There is no such script. <Link to="/scripts">Back to the scripts</Link>.
          </p>
        </>
      ) : context.state === 'loading' ? (
        <>
          <h1 id="script-title">Script</h1>
          <p role="status">Loading…</p>
        </>
      ) : context.state === 'failed' ? (
        <>
          <h1 id="script-title">Script</h1>
          <ErrorState error={context.error} what="Reading your namespaces" onRetry={context.retry} />
        </>
      ) : context.state === 'none' ? (
        <>
          <h1 id="script-title">Script</h1>
          <Banner tone="info">There is no namespace to hold scripts yet.</Banner>
        </>
      ) : (
        <Loader api={context.api} scriptId={scriptId} />
      )}
    </section>
  );
}

function Loader({ api, scriptId }: { api: ScriptsApi; scriptId: string }) {
  // The editor starts from the platform's draft as it is now and keeps its own copy from then on: a
  // cached answer from an earlier visit would carry an old revision, so nothing is kept after the page.
  const query = useQuery({
    queryKey: keys.script(api.namespaceId, scriptId),
    queryFn: ({ signal }) => api.get(scriptId, signal),
    gcTime: 0,
    staleTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  if (query.isError) {
    const missing = isApiError(query.error) && query.error.status === 404;
    return (
      <>
        <h1 id="script-title">{missing ? 'Script not found' : 'Script'}</h1>
        {missing ? (
          <p>
            There is no such script in this namespace, or it is not yours to see. <Link to="/scripts">Back to the scripts</Link>.
          </p>
        ) : (
          <ErrorState error={query.error} what="Reading the script" onRetry={() => void query.refetch()} />
        )}
      </>
    );
  }
  if (query.data === undefined) {
    return (
      <>
        <h1 id="script-title">Script</h1>
        <p role="status">Loading the script…</p>
      </>
    );
  }
  return <Editor key={query.data.id} api={api} script={query.data} />;
}

function Editor({ api, script }: { api: ScriptsApi; script: Script }) {
  const canWrite = useCan('script:write');
  const canPublish = useCan('script:publish');
  const announce = useAnnounce();
  const queries = useQueryClient();
  const language = languageInfo(script.language);

  const [saver] = useState(() => new DraftSaver({ script, update: (body, key) => api.update(script.id, body, key), debounceMs: scriptsConfig.autosaveMs }));
  const draft = useSyncExternalStore(saver.subscribe, saver.getSnapshot);

  const [tab, setTab] = useState('try');
  const [inputText, setInputText] = useState(SAMPLE_INPUT);
  const [viewing, setViewing] = useState<number | undefined>(undefined);
  const [publishing, setPublishing] = useState(false);
  const [problems, setProblems] = useState<Problem[]>([]);
  const controls = useRef<EditorControls | null>(null);

  const versions = useVersionList(api, script.id, true);
  const runs = useQuery({
    queryKey: keys.testRuns(api.namespaceId, script.id),
    queryFn: ({ signal }) => api.testRuns(script.id, { limit: 20 }, signal),
    enabled: canWrite,
    staleTime: 0,
  });
  const refreshRuns = useCallback(() => void queries.invalidateQueries({ queryKey: keys.testRuns(api.namespaceId, script.id) }), [queries, api.namespaceId, script.id]);
  const flush = useCallback(() => saver.save(), [saver]);
  const test = useTestRun({ api, scriptId: script.id, flush, timeoutMs: script.policy.timeoutMs, onFinished: refreshRuns });

  const conflict = draft.state.kind === 'conflict';
  const tested = useMemo(() => (runs.data ? testedness(runs.data.items, draft.updatedAt, draft.unsaved) : undefined), [runs.data, draft.updatedAt, draft.unsaved]);
  const latest = versions.items.reduce<number | undefined>((best, version) => (best === undefined || version.number > best ? version.number : best), script.latestVersion);

  // What went wrong is said aloud once, when it happens; routine saving is not announced.
  const kind = draft.state.kind;
  const lastKind = useRef(kind);
  useEffect(() => {
    if (lastKind.current === kind) return;
    lastKind.current = kind;
    if (kind === 'conflict') announce('Someone else changed this draft. Your changes are kept here and are not saved.');
    else if (kind === 'rejected') announce('The platform refused the draft.');
    else if (kind === 'error') announce('The draft could not be saved.');
  }, [kind, announce]);

  // Leaving the page with edits the platform does not have yet asks first.
  const leaving = draft.unsaved || draft.state.kind === 'saving';
  useEffect(() => {
    if (!leaving) return undefined;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [leaving]);

  const saveNow = () => {
    void saver.save().then((ok) => announce(ok ? 'Draft saved.' : 'The draft was not saved.'));
  };
  const useSchema = (which: 'inputSchema' | 'outputSchema', schema: Record<string, unknown>) => {
    saver.set(which, schemaToText(schema));
    announce(`The inferred schema is now your ${which === 'inputSchema' ? 'input' : 'output'} schema. It is in the schemas tab.`);
  };

  const sourceBytes = useMemo(() => utf8Length(draft.fields.source), [draft.fields.source]);
  const overLimit = sourceBytes > LIMITS.sourceBytes;

  return (
    <>
      <p className="crumbs">
        <Link to="/scripts">All scripts</Link>
      </p>
      <h1 id="script-title">{clipLabel(script.name)}</h1>
      <p className="meta">
        {language.label} · draft revision {draft.revision} · {latest === undefined ? 'not published yet' : `latest version ${latest}`}
      </p>

      {!canWrite ? (
        <Banner tone="info" title="Read only.">
          Your role can read this script but not change it: changing and trying a script needs <code>script:write</code>.
        </Banner>
      ) : (
        <SaveStatus draft={draft} onSave={saveNow} />
      )}
      {conflict ? <ConflictPanel api={api} scriptId={script.id} scriptName={script.name} language={script.language} saver={saver} draft={draft} /> : null}

      {viewing !== undefined ? (
        <VersionView api={api} scriptId={script.id} number={viewing} onBack={() => setViewing(undefined)} />
      ) : (
        <>
          <div className="toolbar">
            {canWrite ? (
              <Button onClick={saveNow} disabled={!draft.unsaved && draft.state.kind === 'saved'}>
                Save now
              </Button>
            ) : null}
            {canPublish ? (
              <Button variant="primary" onClick={() => setPublishing(true)} disabled={conflict}>
                Publish version…
              </Button>
            ) : (
              <span className="hint">Publishing needs the permission script:publish.</span>
            )}
            {tested && canPublish ? <span className="hint">{tested.kind === 'passed' ? 'Tested: the last test run of this draft succeeded.' : 'Not tested since the last change.'}</span> : null}
          </div>

          <section aria-labelledby="source-title" className="source-section">
            <h2 id="source-title">Source</h2>
            <CodeEditor
              label={`Source of ${clipLabel(script.name)} (${language.label})`}
              describedBy="source-size source-checks"
              model="source"
              language={monacoLanguage(script.language)}
              value={draft.fields.source}
              onChange={canWrite ? (text) => saver.set('source', text) : undefined}
              readOnly={!canWrite}
              onProblems={setProblems}
              controlsRef={controls}
              height="26rem"
            />
            <p id="source-size" className={overLimit ? 'error' : 'hint'}>
              {formatBytes(sourceBytes)} of {formatBytes(LIMITS.sourceBytes)}
              {overLimit ? ` — ${formatBytes(sourceBytes - LIMITS.sourceBytes)} over the limit; the platform would refuse it, so it is not saved` : ' available'}.
            </p>
            <p id="source-checks" className="hint">
              {language.checks}
            </p>
            {language.lint ? <ProblemsList problems={problems} controlsRef={controls} label="source" /> : null}
            {draft.problems.length > 0 ? (
              <Banner tone="warn" title="Not saved:">
                <ul>
                  {draft.problems.map((problem) => (
                    <li key={`${problem.field}:${problem.message}`}>{problem.message}</li>
                  ))}
                </ul>
              </Banner>
            ) : null}
          </section>

          <Tabs
            label="Script tools"
            selected={tab}
            onSelect={setTab}
            tabs={[
              {
                id: 'try',
                label: 'Try it in the sandbox',
                panel: canWrite ? (
                  <TryPanel
                    script={script}
                    inputSchemaText={draft.fields.inputSchema}
                    inputText={inputText}
                    onInputText={setInputText}
                    test={test}
                    runs={runs.data?.items}
                    runsError={runs.error}
                    onRetryRuns={() => void runs.refetch()}
                    onUseSchema={useSchema}
                    draftUnsaved={draft.unsaved}
                    draftUpdatedAt={draft.updatedAt}
                  />
                ) : (
                  <p>Trying a script in the sandbox needs the permission script:write.</p>
                ),
              },
              {
                id: 'schemas',
                label: 'Input and output schemas',
                panel: <SchemasPanel inputSchema={draft.fields.inputSchema} outputSchema={draft.fields.outputSchema} readOnly={!canWrite} onChange={(field, text) => saver.set(field, text)} saveState={draft.state} />,
              },
              { id: 'versions', label: 'Versions', panel: <VersionsPanel versions={versions} onOpen={setViewing} /> },
              { id: 'limits', label: 'Limits', panel: <LimitsPanel policy={script.policy} retentionDays={script.retentionDays} language={language.label} /> },
            ]}
          />
        </>
      )}

      <PublishDialog open={publishing} api={api} scriptId={script.id} saver={saver} tested={tested} language={script.language} onClose={() => setPublishing(false)} />
    </>
  );
}

/** Where the draft stands. Routine saving is quiet (a live region that spoke at every pause would be noise); failures are announced elsewhere. */
function SaveStatus({ draft, onSave }: { draft: DraftSnapshot; onSave: () => void }) {
  const { state } = draft;
  let text: string;
  switch (state.kind) {
    case 'saved':
      text = draft.unsaved ? 'Changes not saved: see below.' : 'All changes saved.';
      break;
    case 'dirty':
      text = draft.problems.length > 0 ? 'Changes not saved: see below.' : 'Changes not saved yet; they are saved a moment after you stop typing.';
      break;
    case 'saving':
      text = 'Saving…';
      break;
    case 'conflict':
      text = 'Not saved: someone else changed the draft first. Your changes are kept.';
      break;
    case 'rejected':
      text = 'Not saved: the platform refused the draft.';
      break;
    case 'error':
      text = state.retryInMs === undefined ? 'Not saved: the save failed.' : `Not saved: the save failed. Trying again in ${Math.max(1, Math.round(state.retryInMs / 1000))} s.`;
      break;
  }
  return (
    <div className="save-status">
      <p data-testid="save-status">{text}</p>
      {state.kind === 'error' || state.kind === 'rejected' ? (
        <ErrorState error={state.error} what="Saving the draft" {...(state.kind === 'error' ? { onRetry: onSave } : {})} />
      ) : null}
      {state.kind === 'error' && !(isApiError(state.error) && state.error.retryable) ? <Button onClick={onSave}>Try again</Button> : null}
    </div>
  );
}
