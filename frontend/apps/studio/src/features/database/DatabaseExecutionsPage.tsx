import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
import type { Run } from '../../api/types';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { databaseIssue } from '../../model/database';
import { connect, EditError, removeNode, renameNode, setEntrypoint } from '../../model/edit';
import { parseAuthoringJson } from '../../model/safeJson';
import { emptyDocument } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { NODE_KINDS, kindOf } from '../../model/registry';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { AddNodeDialog } from '../workflows/AddNode';
import { ConnectDialog } from '../workflows/ConnectDialog';
import { NodeInspector } from '../workflows/NodeInspector';
import { NodesTable } from '../workflows/NodesTable';
import { insertNode } from '../workflows/operations';
import { useEditorStore } from '../workflows/useEditorStore';
import { useDatabaseCommand } from './useDatabaseCommand';
import '../../styles/workflows.css';
import '../../styles/database.css';

const KINDS = NODE_KINDS.filter(k => ['database', 'timer', 'condition', 'parallel', 'end'].includes(k.type));
type Request = components['schemas']['StartDatabaseExecutionRequest'];
function initial(): IrDocument {
  const doc = emptyDocument('ad-hoc-database');
  delete doc.triggers;
  return { ...doc, entrypoint: 'database_1', nodes: [kindOf('database')!.make('database_1')] };
}

export function DatabaseExecutionsPage() {
  const { ep } = useApp(); const slug = useNamespace(); const { info } = useSession();
  const adhoc = useCan('database:adhoc'); const use = useCan('database:use'); const start = useCan('run:start');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find(n => n.slug === slug)?.id;
  if (!adhoc || !use || !start) return <section><h1>Ad hoc database execution</h1><p>Requires database:adhoc, database:use and run:start permissions.</p></section>;
  if (namespaces.error) return <ErrorState error={namespaces.error} what="Loading the namespace" />;
  if (!namespace) return <p role="status">Select a namespace to execute database work.</p>;
  return <ExecutionEditor key={`${info?.tenant}:${info?.subject}:${namespace}`} namespace={namespace} />;
}

function ExecutionEditor({ namespace }: { namespace: string }) {
  const { api, session } = useApp(); const canWrite = useCan('database:write');
  const command = useDatabaseCommand<Request>((intent, signal) => api.post<Run>(path`/namespaces/${namespace}/database-executions`, { ...intent, signal }), session, `adhoc:${namespace}`);
  const retained = command.body;
  const { store, doc, selectedId, canUndo, canRedo } = useEditorStore(retained ? retained.definition as IrDocument : initial());
  const [mode, setMode] = useState<'step' | 'workflow'>(retained?.mode ?? 'step');
  const [input, setInput] = useState(() => JSON.stringify(retained?.input ?? {})); const [timeout, setTimeout] = useState(() => String(retained?.timeoutMs ?? 300000));
  const [add, setAdd] = useState(false); const [from, setFrom] = useState<string | undefined>();
  const [notice, setNotice] = useState('');
  const ambiguous = command.state.phase === 'sending' || command.state.phase === 'failed' && !command.state.refused;
  useEffect(() => {
    if (!ambiguous) return;
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [ambiguous]);
  let parsed: Record<string, unknown> | undefined;
  try { const value: unknown = parseAuthoringJson(input); if (value !== null && typeof value === 'object' && !Array.isArray(value)) parsed = value as Record<string, unknown>; } catch { /* Explicit error below, no stale input. */ }
  const timeoutValid = /^\d+$/.test(timeout) && Number(timeout) >= 1 && Number(timeout) <= 3_600_000;
  const problems = new Map(doc.nodes.filter(n => n.type === 'database' && databaseIssue(n) !== undefined).map(n => [n.id, 1]));
  const hasDatabase = doc.nodes.some(n => n.type === 'database');
  const forbiddenWrite = !canWrite && doc.nodes.some(n => n['operation'] === 'execute');
  const selected = doc.nodes.find(n => n.id === selectedId) ?? doc.nodes[0];
  const apply = (fn: (d: IrDocument) => IrDocument, key?: string, select?: string): string | undefined => {
    if (command.locked) return 'This request is fixed until its outcome is known.';
    try { store.edit(fn, { ...(key ? { key } : {}), ...(select ? { select } : {}) }); setNotice(''); return undefined; }
    catch (error) { if (!(error instanceof EditError)) throw error; setNotice(error.message); return error.message; }
  };
  if (command.state.phase === 'accepted') return <section><h1>Execution accepted</h1><p role="status">The definition and run have been retained. Closing this page does not stop the run.</p><p><Link className="btn primary" to={path`/runs/${command.state.run.runId}`}>Open run</Link></p><p><Button onClick={() => { store.reset(initial()); setMode('step'); setInput('{}'); command.reset(); }}>New execution</Button></p><p><Link to="/connections">Back to connections</Link></p></section>;
  return <section className="database-execution" aria-labelledby="database-execution-title">
    <header><h1 id="database-execution-title">Ad hoc database execution</h1><p>Run a single step or a workflow using the connections in this namespace.</p><p className="hint">The platform retains each submitted definition and run. This editor keeps unsent SQL and inputs only on this page.</p><Link to="/runs">Find previous executions in Runs</Link></header>
    <fieldset disabled={command.locked} className="database-editor-fields">
      <legend className="visually-hidden">Execution definition</legend>
      <div className="toolbar"><Field label="Execution mode"><select value={mode} onChange={e => setMode(e.target.value as 'step' | 'workflow')}><option value="step" disabled={doc.nodes.length !== 1 || doc.nodes[0]?.type !== 'database'}>Single database step</option><option value="workflow">Ad hoc workflow</option></select></Field>
        <Button disabled={!canUndo || command.locked} onClick={() => store.undo()}>Undo</Button><Button disabled={!canRedo || command.locked} onClick={() => store.redo()}>Redo</Button>
        {mode === 'workflow' ? <Button disabled={doc.nodes.length >= 64 || command.locked} onClick={() => setAdd(true)}>Add node</Button> : null}
      </div>
      {notice ? <p role="alert">{notice}</p> : null}
      {mode === 'workflow' ? <NodesTable doc={doc} selectedId={selected?.id} problems={problems} advice={new Map()} editable={!command.locked} caption="Ad hoc workflow nodes" onEdit={id => store.select(id)} onDelete={id => void apply(d => removeNode(d, id).doc)} onSetFirst={id => void apply(d => setEntrypoint(d, id))} onConnect={id => setFrom(id)} /> : null}
      <div className="database-editor-grid"><div className="database-node-editor">{selected ? <NodeInspector doc={doc} node={selected} disabled={command.locked} notes={[]} actions={{ change: (fn, key) => void apply(fn, key), rename: (a, b) => apply(d => renameNode(d, a, b), undefined, b), setFirst: id => void apply(d => setEntrypoint(d, id)), remove: id => void apply(d => removeNode(d, id).doc) }} /> : <p>Add a database node to begin.</p>}</div>
        <div className="database-run-input"><h2>Run settings</h2><Field label="Run input (JSON)" error={parsed ? undefined : 'Enter a JSON object. Use strings for exact integers and decimals.'} hint="Use expressions such as input.orderId in parameter bindings. Do not put credentials here."><textarea rows={8} spellCheck={false} autoComplete="off" maxLength={262_144} value={input} onChange={e => setInput(e.target.value)} /></Field>
          <Field label="Run timeout (ms)" error={timeoutValid ? undefined : 'Enter a whole number from 1 to 3600000.'}><input inputMode="numeric" autoComplete="off" value={timeout} onChange={e => setTimeout(e.target.value)} /></Field>
          <p className="hint">Query results up to 256 KiB can be read by downstream expressions. Larger results are retained as blob references.</p>
        </div>
      </div>
    </fieldset>
    {!hasDatabase ? <Banner tone="warn">Add at least one database node.</Banner> : null}
    {forbiddenWrite ? <Banner tone="warn">Your role cannot execute database writes.</Banner> : null}
    {command.state.phase === 'failed' ? <div><ErrorState error={command.state.error} what="Starting the execution" /><Banner tone="warn">{command.state.refused ? 'The request was refused. You can edit it or retry.' : 'The run may already exist. Retry this same request to recover its run ID. Check Runs before submitting another execution.'}</Banner><Button onClick={() => void command.send()}>Retry same execution</Button>{command.state.refused ? <Button onClick={command.reset}>Edit refused request</Button> : null}</div> : null}
    <Button variant="primary" disabled={command.locked || !parsed || !timeoutValid || !hasDatabase || problems.size > 0 || forbiddenWrite || mode === 'step' && doc.nodes.length !== 1} onClick={() => { if (parsed) void command.send({ mode, definition: doc, input: parsed, timeoutMs: Number(timeout) }); }}>{command.state.phase === 'sending' ? 'Starting execution…' : 'Run execution'}</Button>
    <AddNodeDialog open={add && !command.locked} kinds={KINDS} where="Adds a node after the selected node when possible. At most 64 nodes." onClose={() => setAdd(false)} onPick={type => { const added = insertNode(doc, type, selected?.id); void apply(() => added.doc, undefined, added.id); }} />
    <ConnectDialog open={from !== undefined && !command.locked} doc={doc} from={from} onClose={() => setFrom(undefined)} onConnect={(field, to) => from ? apply(d => connect(d, from, field, to)) : 'Choose a node.'} />
  </section>;
}
