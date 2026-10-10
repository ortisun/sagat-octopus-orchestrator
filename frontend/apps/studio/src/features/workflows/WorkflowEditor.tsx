import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useContext, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { FocusEvent } from 'react';
import { isApiError } from '../../api/errors';
import type { Workflow, WorkflowVersion } from '../../api/types';
import { useApp, useCan } from '../../app/context';
import { EditError, connect, removeNode, renameNode, setEntrypoint } from '../../model/edit';
import { advise } from '../../model/advice';
import { canonical } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { kindOf } from '../../model/registry';
import type { Direction, Point } from '../../model/layout';
import { Banner } from '../../ui/Banner';
import { useAnnounce } from '../../ui/Announcer';
import { Button } from '../../ui/Button';
import { Tabs } from '../../ui/Tabs';
import { AddNodeDialog, Palette } from './AddNode';
import { canvasMode } from './canvasModel';
import { ConflictDialog } from './ConflictDialog';
import type { CopyResult } from './ConflictDialog';
import { ConnectDialog } from './ConnectDialog';
import { DraftSession } from './draftSession';
import { openDraft, toWire } from './documentIO';
import { KeyboardHelp } from './KeyboardHelp';
import { LeaveGuard } from './LeaveGuard';
import { NodeInspector } from './NodeInspector';
import type { NodeNote } from './NodeInspector';
import { NodesTable } from './NodesTable';
import { copyName, insertNode } from './operations';
import { PublishDialog } from './PublishDialog';
import { TimingsContext } from './timings';
import { useEditorStore } from './useEditorStore';
import { useValidation } from './useValidation';
import { ValidationPanel } from './ValidationPanel';
import type { ValidationPanelHandle } from './ValidationPanel';
import { countsByNode, summaryLine } from './validationModel';
import { readDirection, readPositions, readView, writeDirection, writePositions, writeView } from './viewPrefs';
import type { Positions, ViewMode } from './viewPrefs';
import { VersionsPanel } from './VersionsPanel';
import { WorkflowCanvas } from './WorkflowCanvas';
import type { CanvasApi } from './WorkflowCanvas';
import { WorkflowSettings } from './WorkflowSettings';
import '../../styles/workflows.css';

/** Typing in a field is not a place to undo the document, a click on a card is not typing. */
function isTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  return target instanceof HTMLInputElement && !['checkbox', 'radio', 'button', 'submit'].includes(target.type);
}

function errorText(error: unknown): string {
  return isApiError(error) ? error.message : 'Something went wrong.';
}

interface Notice {
  text: string;
  n: number;
}

export function WorkflowEditor({ workflow }: { workflow: Workflow }) {
  const { ep } = useApp();
  const announce = useAnnounce();
  const queries = useQueryClient();
  const timings = useContext(TimingsContext);
  const canWrite = useCan('definition:write');
  const canPublish = useCan('definition:publish');

  const [opened] = useState(() => openDraft(workflow.ir));
  const [extras, setExtras] = useState<readonly unknown[]>(opened.extras);
  const { store, doc, selectedId, canUndo, canRedo } = useEditorStore(opened.doc);
  const wire = useMemo(() => toWire(doc, extras), [doc, extras]);
  const wireKey = useMemo(() => canonical(wire), [wire]);
  const [draft] = useState(() => new DraftSession({ ep, id: workflow.id, revision: workflow.draftRevision, wire: toWire(opened.doc, opened.extras) }));
  const save = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  useLayoutEffect(() => {
    draft.update(wire, wireKey);
  }, [draft, wire, wireKey]);

  // ------------------------------------------------------------------ what the checks say
  const validation = useValidation(ep, workflow.id, wire, wireKey, timings.validateMs, canWrite);
  const deferred = useDeferredValue(doc);
  const advice = useMemo(() => advise(deferred), [deferred]);
  const problemCounts = useMemo(() => countsByNode(validation.view?.problems ?? []), [validation.view]);
  const adviceCounts = useMemo(() => countsByNode(advice.map((a) => ({ nodeId: a.nodeId }))), [advice]);
  const problems = validation.view?.problems.length ?? 0;
  const warnings = validation.view?.warnings.length ?? 0;
  const summary = summaryLine(problems, warnings, advice.length);

  // ------------------------------------------------------------------ view preferences
  const [view, setView] = useState<ViewMode>(readView);
  const [direction, setDirection] = useState<Direction>(readDirection);
  const [manual, setManual] = useState<Positions>(() => readPositions(workflow.id));
  const [showAnyway, setShowAnyway] = useState(false);
  const [fitSignal, setFitSignal] = useState(0);
  const mode = canvasMode(doc.nodes.length);
  const showList = view === 'list' || (mode === 'list' && !showAnyway);
  useEffect(() => {
    const live = new Set(['', ...doc.nodes.map((n) => n.id)]);
    const kept: Positions = new Map([...manual].filter(([id]) => live.has(id) || id.startsWith('@trigger:')));
    writePositions(workflow.id, kept);
  }, [manual, doc.nodes, workflow.id]);

  // ------------------------------------------------------------------ small state
  const [notice, setNotice] = useState<Notice>({ text: '', n: 0 });
  const say = useCallback((text: string) => setNotice((previous) => ({ text, n: previous.n + 1 })), []);
  // What was said about the last change is about that change: it goes away, so it is never read as news later.
  useEffect(() => {
    if (notice.text === '') return;
    const timer = setTimeout(() => setNotice((current) => (current.n === notice.n ? { text: '', n: current.n } : current)), timings.noticeMs ?? 10_000);
    return () => clearTimeout(timer);
  }, [notice, timings.noticeMs]);
  const [addOpen, setAddOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [publishOpen, setPublishOpen] = useState(false);
  const [connectFrom, setConnectFrom] = useState<string | undefined>();
  const [dockOpen, setDockOpen] = useState(true);
  const [dockTab, setDockTab] = useState('validation');
  const [validationFocus, setValidationFocus] = useState(0);
  const [inspectorFocus, setInspectorFocus] = useState(0);
  const [focusId, setFocusId] = useState<string | undefined>();
  const [dismissedTheirs, setDismissedTheirs] = useState<Workflow | undefined>();
  const [copied, setCopied] = useState<CopyResult | undefined>();
  const [active, setActive] = useState<number | undefined>(workflow.activeVersion?.versionNumber);
  const [published, setPublished] = useState<WorkflowVersion | undefined>();
  const canvasApi = useRef<CanvasApi | null>(null);
  const validationPanel = useRef<ValidationPanelHandle>(null);
  const inspectorHeading = useRef<HTMLHeadingElement>(null);
  const tableHeading = useRef<HTMLHeadingElement>(null);

  const nodeExists = useCallback((id: string) => doc.nodes.some((n) => n.id === id), [doc.nodes]);
  // The card that Tab reaches: the last one visited if it is still there, else the selected one, else where the run starts.
  const rovingId = (() => {
    const there = (id: string | undefined) => id !== undefined && (nodeExists(id) || (id.startsWith('@trigger:') && Number(id.slice(9)) < (doc.triggers ?? []).length));
    if (there(focusId)) return focusId;
    if (there(selectedId)) return selectedId;
    return there(doc.entrypoint) ? doc.entrypoint : doc.nodes[0]?.id;
  })();
  const selectedNode = selectedId === undefined ? undefined : doc.nodes.find((n) => n.id === selectedId);

  useEffect(() => {
    if (inspectorFocus > 0) inspectorHeading.current?.focus();
  }, [inspectorFocus]);
  useEffect(() => {
    if (validationFocus > 0) validationPanel.current?.focus();
  }, [validationFocus]);

  // ------------------------------------------------------------------ editing
  /** Runs a change of the document; a refused one changes nothing and says why. Returns the reason, if refused. */
  const apply = useCallback(
    (change: (d: IrDocument) => IrDocument, options?: { key?: string; select?: string | null }): string | undefined => {
      if (!canWrite) return 'You can read this workflow but not change it.';
      try {
        store.edit(change, options);
        return undefined;
      } catch (error) {
        if (error instanceof EditError) {
          say(`Not changed. ${error.message}`);
          return error.message;
        }
        throw error;
      }
    },
    [canWrite, store, say],
  );

  const change = useCallback((fn: (d: IrDocument) => IrDocument, key: string) => void apply(fn, { key }), [apply]);

  const placeAt = useCallback((id: string, at: Point | undefined) => {
    if (at) setManual((previous) => new Map(previous).set(id, at));
  }, []);

  const addNodeOfKind = useCallback(
    (type: string, at?: Point) => {
      const inserted = insertNode(store.doc(), type, at ? undefined : selectedId);
      const reason = apply(() => inserted.doc, { select: inserted.id });
      if (reason !== undefined) return;
      const place = at ?? (selectedId === undefined && !showList ? canvasApi.current?.viewportCenter() : undefined);
      placeAt(inserted.id, place);
      say(inserted.note);
    },
    [apply, placeAt, say, selectedId, showList, store],
  );

  const deleteNode = useCallback(
    (id: string) => {
      const label = kindOf(store.doc().nodes.find((n) => n.id === id)?.type ?? '')?.label ?? 'node';
      let cleared = 0;
      let reading = 0;
      const reason = apply(
        (d) => {
          const removed = removeNode(d, id);
          cleared = removed.cleared.length;
          reading = removed.dangling.length;
          return removed.doc;
        },
        { select: null },
      );
      if (reason === undefined) {
        const readers = reading > 0 ? ` ${reading === 1 ? 'An expression still reads' : `${reading} expressions still read`} its output: see the advice.` : '';
        say(`Deleted ${label} "${id}"${cleared > 0 ? ` and ${cleared} ${cleared === 1 ? 'link' : 'links'} to it` : ''}.${readers} Undo brings it back.`);
        if (showList) tableHeading.current?.focus();
      }
    },
    [apply, say, showList, store],
  );

  const renameTo = useCallback(
    (from: string, to: string): string | undefined => {
      const reason = apply((d) => renameNode(d, from, to), { select: to });
      if (reason === undefined) {
        setManual((previous) => {
          const at = previous.get(from);
          if (at === undefined) return previous;
          const next = new Map(previous);
          next.delete(from);
          return next.set(to, at);
        });
        say(`Renamed "${from}" to "${to}". The links to it, and the expressions that read its output, follow.`);
      }
      return reason;
    },
    [apply, say],
  );

  const connectNodes = useCallback(
    (from: string, field: string, to: string): string | undefined => {
      const reason = apply((d) => connect(d, from, field, to));
      if (reason === undefined) {
        const label = kindOf(store.doc().nodes.find((n) => n.id === from)?.type ?? '')?.handles.find((h) => h.field === field)?.label ?? field;
        say(`Connected "${from}" to "${to}" (${label}).`);
      }
      return reason;
    },
    [apply, say, store],
  );

  const setFirst = useCallback(
    (id: string) => {
      if (apply((d) => setEntrypoint(d, id)) === undefined) say(`"${id}" is now the first step.`);
    },
    [apply, say],
  );

  const undo = useCallback(() => say(store.undo() ? 'Undone.' : 'Nothing to undo.'), [say, store]);
  const redo = useCallback(() => say(store.redo() ? 'Redone.' : 'Nothing to redo.'), [say, store]);

  const openInspector = useCallback(
    (id: string) => {
      store.select(id);
      setInspectorFocus((n) => n + 1);
    },
    [store],
  );

  // ------------------------------------------------------------------ saving
  const saveNow = useCallback(async () => {
    if (!canWrite) return;
    if (!draft.getSnapshot().dirty && draft.getSnapshot().phase !== 'failed') {
      announce('Nothing to save.');
      return;
    }
    const outcome = await draft.save();
    const now = draft.getSnapshot();
    if (outcome === 'saved') announce(`Saved. Revision ${now.revision}.`);
    else if (outcome === 'failed') announce(`The draft could not be saved. ${errorText(now.error)}`);
  }, [announce, canWrite, draft]);

  useEffect(() => {
    if (!canWrite || !save.dirty || save.phase === 'saving' || save.phase === 'conflict') return;
    if (save.phase === 'failed' && save.failedKey === save.currentKey) return;
    const timer = setTimeout(() => void draft.save(), timings.autosaveMs);
    return () => clearTimeout(timer);
  }, [canWrite, draft, save.currentKey, save.dirty, save.failedKey, save.phase, timings.autosaveMs]);

  const saveOnLeave = useCallback(() => {
    if (canWrite) void draft.save();
  }, [canWrite, draft]);
  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget)) saveOnLeave();
  };

  const unsaved = canWrite && (save.dirty || save.phase === 'saving');
  useEffect(() => {
    if (!unsaved) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [unsaved]);
  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === 'hidden') saveOnLeave();
    };
    document.addEventListener('visibilitychange', hidden);
    return () => document.removeEventListener('visibilitychange', hidden);
  }, [saveOnLeave]);

  // ------------------------------------------------------------------ conflict
  const theirs = save.phase === 'conflict' ? save.theirs : undefined;
  const takeTheirs = useCallback(() => {
    const snapshot = draft.getSnapshot();
    if (!snapshot.theirs) return;
    const taken = openDraft(snapshot.theirs.ir);
    store.reset(taken.doc);
    setExtras(taken.extras);
    draft.adopt(snapshot.theirs, toWire(taken.doc, taken.extras));
    announce(`Took the other draft, revision ${snapshot.theirs.draftRevision}. Your changes were discarded.`);
  }, [announce, draft, store]);
  const keepMine = useCallback(async () => {
    const snapshot = draft.getSnapshot();
    if (!snapshot.theirs) return;
    const outcome = await draft.overwrite(snapshot.theirs);
    if (outcome === 'failed') throw draft.getSnapshot().error;
    if (outcome === 'saved') announce('Saved your version over their draft.');
  }, [announce, draft]);
  const saveCopy = useCallback(async (): Promise<void> => {
    const mine = store.doc();
    const mineWire = toWire(mine, extras);
    let last: unknown;
    for (let attempt = 0; attempt < 5; attempt++) {
      const name = copyName(mine.metadata.name || workflow.name, attempt);
      try {
        const created = await ep.createWorkflow(name, { ...mineWire, metadata: { ...mine.metadata, name } });
        takeTheirs();
        setCopied({ id: created.id, name });
        void queries.invalidateQueries({ queryKey: ['workflows'] });
        return;
      } catch (error) {
        last = error;
        if (!(isApiError(error) && error.code === 'CONFLICT')) throw error;
      }
    }
    throw last;
  }, [ep, extras, queries, store, takeTheirs, workflow.name]);

  // ------------------------------------------------------------------ publishing
  const blockedReason = (() => {
    if (problems > 0) return `Publishing is blocked: the platform found ${problems} ${problems === 1 ? 'problem' : 'problems'} in this workflow.`;
    if (!validation.current && validation.phase === 'checking') return 'Publishing waits for the platform to finish checking the workflow.';
    if (save.phase === 'conflict') return 'Publishing is blocked until the conflict with the other draft is resolved.';
    return undefined;
  })();
  const showProblems = useCallback(() => {
    setDockOpen(true);
    setDockTab('validation');
    setValidationFocus((n) => n + 1);
  }, []);
  const onPublishClick = () => {
    if (blockedReason !== undefined) {
      announce(blockedReason);
      if (problems > 0) showProblems();
      return;
    }
    setPublishOpen(true);
  };
  const onPublished = useCallback(
    (version: WorkflowVersion) => {
      setPublishOpen(false);
      setActive(version.versionNumber);
      setPublished(version);
      announce(`Published version ${version.versionNumber}.`);
      void queries.invalidateQueries({ queryKey: ['workflow-versions'] });
      void queries.invalidateQueries({ queryKey: ['workflows'] });
    },
    [announce, queries],
  );
  const onPublishConflict = useCallback(
    (other: Workflow) => {
      draft.conflictWith(other);
      setPublishOpen(false);
    },
    [draft],
  );
  const closePublish = useCallback(() => setPublishOpen(false), []);
  const currentKey = useCallback(() => canonical(toWire(store.doc(), extras)), [extras, store]);

  // ------------------------------------------------------------------ shortcuts
  const shortcutState = useRef({ canWrite, undo, redo, saveNow });
  useEffect(() => {
    shortcutState.current = { canWrite, undo, redo, saveNow };
  });
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      // An open dialog owns the keyboard; the browser keeps focus inside it, and nothing behind it is for now.
      if (document.querySelector('dialog[open]')) return;
      const now = shortcutState.current;
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      const typing = isTextTarget(target);
      if (mod && !event.altKey && key === 's') {
        event.preventDefault();
        void now.saveNow();
      } else if (event.altKey && !mod && event.code === 'KeyN') {
        if (now.canWrite) {
          event.preventDefault();
          setAddOpen(true);
        }
      } else if (mod && !event.altKey && !typing && now.canWrite && (key === 'z' || key === 'y')) {
        event.preventDefault();
        if (key === 'y' || event.shiftKey) now.redo();
        else now.undo();
      } else if (event.key === '?' && !mod && !event.altKey && !typing) {
        event.preventDefault();
        setHelpOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // ------------------------------------------------------------------ announcements nobody else makes
  const notes = useMemo((): NodeNote[] => {
    if (selectedId === undefined) return [];
    return [
      ...(validation.view?.problems ?? []).filter((p) => p.nodeId === selectedId).map((p): NodeNote => ({ kind: 'problem', text: p.message })),
      ...(validation.view?.warnings ?? []).filter((p) => p.nodeId === selectedId).map((p): NodeNote => ({ kind: 'warning', text: p.message })),
      ...advice.filter((a) => a.nodeId === selectedId).map((a): NodeNote => ({ kind: 'advice', text: a.message })),
    ];
  }, [advice, selectedId, validation.view]);

  // ------------------------------------------------------------------ view handlers
  const chooseView = (next: ViewMode) => {
    setView(next);
    if (next === 'canvas') setShowAnyway(true);
    if (!(next === 'canvas' && mode === 'list')) writeView(next);
  };
  const chooseDirection = (next: Direction) => {
    setDirection(next);
    writeDirection(next);
    setManual(new Map());
    setFitSignal((n) => n + 1);
  };
  const relayout = () => {
    setManual(new Map());
    setFitSignal((n) => n + 1);
    say('Cards were laid out again.');
  };
  const onReady = useCallback((api: CanvasApi | null) => {
    canvasApi.current = api;
  }, []);
  const onMoved = useCallback((id: string, at: Point) => setManual((previous) => new Map(previous).set(id, at)), []);
  const onSelectCard = useCallback((id: string | undefined) => store.select(id), [store]);
  const onOpenTriggers = useCallback(() => {
    store.select(undefined);
    setInspectorFocus((n) => n + 1);
  }, [store]);
  const onStartAt = useCallback((to: string) => setFirst(to), [setFirst]);
  const onConnectCards = useCallback((from: string, field: string, to: string) => void connectNodes(from, field, to), [connectNodes]);
  const onDropKind = useCallback((type: string, at: Point | undefined) => addNodeOfKind(type, at), [addNodeOfKind]);

  const readOnly = !canWrite;
  const saveLabel = (() => {
    if (readOnly) return { text: 'Read only', shape: '•', state: 'readonly' };
    if (save.phase === 'saving') return { text: 'Saving…', shape: '…', state: 'saving' };
    if (save.phase === 'conflict') return { text: 'Conflict', shape: '!', state: 'conflict' };
    if (save.phase === 'failed') {
      const trace = isApiError(save.error) && save.error.traceId ? ` (trace ${save.error.traceId})` : '';
      return { text: `Could not save${trace}`, shape: '✕', state: 'failed' };
    }
    if (save.dirty) return { text: 'Unsaved changes', shape: '●', state: 'dirty' };
    return { text: `Saved · revision ${save.revision}`, shape: '✓', state: 'saved' };
  })();

  const addWhere =
    selectedId !== undefined
      ? `The new node goes after "${selectedId}", in its first free way out.`
      : showList
        ? 'The new node is added at the end of the list; link it from the inspector or with Connect.'
        : 'The new node goes to the middle of the visible canvas.';

  return (
    <div className="wf-editor" onBlur={onBlur} data-testid="editor">
      <div className="wf-head">
        <h1>{doc.metadata.name || workflow.name}</h1>
        {active !== undefined ? (
          <span className="badge" data-tone="ok">
            <span aria-hidden="true">✓</span>
            <span>Active version {active}</span>
          </span>
        ) : (
          <span className="badge" data-tone="idle">
            <span>Not published</span>
          </span>
        )}
      </div>

      {readOnly ? <Banner tone="info">You can read this workflow but not change it: editing needs the definition:write permission.</Banner> : null}
      {opened.unreadable.length > 0 ? (
        <Banner tone="warn">
          {opened.unreadable.length} {opened.unreadable.length === 1 ? 'entry' : 'entries'} of &quot;nodes&quot; in this draft cannot be read by the Studio (
          {opened.unreadable.slice(0, 5).join(', ')}
          {opened.unreadable.length > 5 ? ', …' : ''}). {opened.unreadable.length === 1 ? 'It is' : 'They are'} kept as {opened.unreadable.length === 1 ? 'it is' : 'they are'} when
          you save, and the platform&apos;s validation says what is wrong.
        </Banner>
      ) : null}
      {published ? (
        <Banner tone="ok">
          {published.created ? `Published version ${published.versionNumber}.` : `This draft is identical to version ${published.versionNumber}, which is active again.`}
          {published.warnings.length > 0 ? ` The platform noted: ${published.warnings.map((w) => w.slice(0, 200)).join(' ')}` : ''}{' '}
          <Button onClick={() => setPublished(undefined)}>Dismiss</Button>
        </Banner>
      ) : null}

      <div className="wf-toolbar" role="group" aria-label="Editor tools">
        <div role="group" aria-label="View" className="wf-toggle">
          <Button aria-pressed={!showList} onClick={() => chooseView('canvas')}>
            Canvas
          </Button>
          <Button aria-pressed={showList} onClick={() => chooseView('list')}>
            List
          </Button>
        </div>
        {canWrite ? (
          <>
            <Button onClick={() => setAddOpen(true)}>Add node</Button>
            <Button onClick={undo} disabled={!canUndo} aria-keyshortcuts="Control+Z Meta+Z">
              Undo
            </Button>
            <Button onClick={redo} disabled={!canRedo} aria-keyshortcuts="Control+Shift+Z Meta+Shift+Z Control+Y">
              Redo
            </Button>
          </>
        ) : null}
        {!showList ? (
          <>
            <Button
              onClick={() => chooseDirection(direction === 'LR' ? 'TB' : 'LR')}
              aria-label={`Layout direction: ${direction === 'LR' ? 'left to right' : 'top to bottom'}. Switch to ${direction === 'LR' ? 'top to bottom' : 'left to right'}.`}
            >
              {direction === 'LR' ? 'Left to right' : 'Top to bottom'}
            </Button>
            <Button onClick={relayout}>Re-layout</Button>
          </>
        ) : null}
        <span className="spacer" />
        <span role="status" className="wf-summary" data-testid="validation-summary">
          {validation.view ? summary : ''}
        </span>
        <span className="wf-save" data-state={saveLabel.state}>
          <span aria-hidden="true">{saveLabel.shape}</span> <span data-testid="save-state">{saveLabel.text}</span>
        </span>
        {canWrite ? <Button onClick={() => void saveNow()}>Save now</Button> : null}
        {save.phase === 'conflict' && dismissedTheirs === theirs ? <Button onClick={() => setDismissedTheirs(undefined)}>Resolve conflict…</Button> : null}
        <Button onClick={() => setHelpOpen(true)} aria-keyshortcuts="?">
          Keyboard help
        </Button>
        {canPublish ? (
          <>
            <Button
              variant="primary"
              onClick={onPublishClick}
              aria-disabled={blockedReason !== undefined ? true : undefined}
              aria-describedby={blockedReason !== undefined ? 'wf-publish-reason' : undefined}
              title={blockedReason}
            >
              Publish…
            </Button>
          </>
        ) : null}
      </div>
      {canPublish && blockedReason !== undefined ? (
        <p id="wf-publish-reason" className="wf-publish-reason">
          {blockedReason} {problems > 0 ? <Button onClick={showProblems}>Show the problems</Button> : null}
        </p>
      ) : null}

      <div className="wf-notice" role="status" data-testid="edit-notice">
        <span key={notice.n}>{notice.text}</span>
      </div>

      <div className="wf-body">
        {!showList && canWrite ? <Palette onPick={(type) => addNodeOfKind(type)} /> : null}
        <div className="wf-main">
          {showList ? (
            <section aria-labelledby="wf-nodes-heading">
              <h2 id="wf-nodes-heading" className="wf-panel-title" ref={tableHeading} tabIndex={-1}>
                Nodes
              </h2>
              {mode === 'list' && !showAnyway ? (
                <Banner tone="warn" title="A large workflow.">
                  This workflow has {doc.nodes.length.toLocaleString()} nodes, more than a canvas draws comfortably, so it opens as a list.{' '}
                  <Button
                    onClick={() => {
                      setShowAnyway(true);
                      setView('canvas');
                    }}
                  >
                    Show the canvas anyway
                  </Button>
                </Banner>
              ) : null}
              <NodesTable
                doc={doc}
                selectedId={selectedId}
                problems={problemCounts}
                advice={adviceCounts}
                editable={canWrite}
                caption="Nodes of the workflow"
                onEdit={openInspector}
                onDelete={deleteNode}
                onSetFirst={setFirst}
                onConnect={setConnectFrom}
              />
            </section>
          ) : (
            <>
              {mode === 'simplified' || mode === 'list' ? (
                <Banner tone="info">With {doc.nodes.length.toLocaleString()} nodes the cards are simplified (id and type only). The list view shows everything.</Banner>
              ) : null}
              {doc.nodes.length === 0 ? <p className="wf-empty-hint">This workflow is empty. Use Add node (Alt+N), or drag a kind from the palette onto the canvas.</p> : null}
              <WorkflowCanvas
                doc={doc}
                mode={mode === 'full' ? 'full' : 'simplified'}
                direction={direction}
                manual={manual}
                selectedId={selectedId}
                focusId={rovingId}
                problems={problemCounts}
                advice={adviceCounts}
                editable={canWrite}
                fitSignal={fitSignal}
                onReady={onReady}
                onSelect={onSelectCard}
                onOpen={openInspector}
                onFocusCard={setFocusId}
                onMoved={onMoved}
                onConnect={onConnectCards}
                onStartAt={onStartAt}
                onDelete={deleteNode}
                onDropKind={onDropKind}
                onOpenTriggers={onOpenTriggers}
              />
            </>
          )}
        </div>

        {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- the inspector scrolls, and for a reader who cannot edit it has nothing else to focus (WCAG 2.1.1) */}
        <aside className="wf-inspector" aria-label="Inspector" tabIndex={0}>
          {selectedNode ? (
            <NodeInspector
              ref={inspectorHeading}
              doc={doc}
              node={selectedNode}
              disabled={!canWrite}
              notes={notes}
              actions={{ change, rename: renameTo, setFirst, remove: deleteNode }}
            />
          ) : (
            <WorkflowSettings ref={inspectorHeading} doc={doc} disabled={!canWrite} change={change} />
          )}
        </aside>
      </div>

      <section className="wf-dock" aria-label="Checks and versions">
        <div className="wf-dock-head">
          <Button aria-expanded={dockOpen} aria-controls="wf-dock-body" onClick={() => setDockOpen((open) => !open)}>
            Validation and versions
          </Button>
          <span className="hint">
            {validation.phase === 'checking'
              ? validation.view
                ? 'Checking again…'
                : 'Checking with the platform…'
              : validation.phase === 'failed'
                ? 'The platform could not check this just now.'
                : ''}
          </span>
        </div>
        {dockOpen ? (
          <div id="wf-dock-body" className="wf-dock-body">
            <Tabs
              label="Checks and versions"
              selected={dockTab}
              onSelect={setDockTab}
              tabs={[
                {
                  id: 'validation',
                  label: 'Validation',
                  panel: (
                    <ValidationPanel
                      ref={validationPanel}
                      state={validation}
                      advice={advice}
                      exists={nodeExists}
                      onSelectNode={(id) => store.select(id)}
                      checkedSummary={validation.phase === 'off' ? 'The platform checks a workflow for the people who can change it (definition:write). You can read it, so it is not checked for you.' : validation.view ? summary : 'Waiting for the first answer of the platform.'}
                    />
                  ),
                },
                {
                  id: 'versions',
                  label: 'Versions',
                  panel: <VersionsPanel workflowId={workflow.id} activeNumber={active} draftDirty={save.dirty} draftRevision={save.revision} />,
                },
              ]}
            />
          </div>
        ) : null}
      </section>

      <AddNodeDialog open={addOpen} where={addWhere} onClose={() => setAddOpen(false)} onPick={(type) => addNodeOfKind(type)} />
      <KeyboardHelp open={helpOpen} onClose={() => setHelpOpen(false)} />
      <ConnectDialog
        open={connectFrom !== undefined}
        doc={doc}
        from={connectFrom}
        onClose={() => setConnectFrom(undefined)}
        onConnect={(field, to) => (connectFrom === undefined ? undefined : connectNodes(connectFrom, field, to))}
      />
      <PublishDialog
        open={publishOpen}
        ep={ep}
        workflowId={workflow.id}
        session={draft}
        currentKey={currentKey}
        onClose={closePublish}
        onPublished={onPublished}
        onConflict={onPublishConflict}
      />
      <ConflictDialog
        open={copied !== undefined || (theirs !== undefined && dismissedTheirs !== theirs)}
        theirs={theirs}
        copied={copied}
        mine={doc}
        onClose={() => (copied ? setCopied(undefined) : setDismissedTheirs(theirs))}
        onTakeTheirs={takeTheirs}
        onKeepMine={keepMine}
        onSaveCopy={saveCopy}
      />
      <LeaveGuard active={unsaved} onSave={async () => (await draft.save()) === 'saved'} />
    </div>
  );
}
