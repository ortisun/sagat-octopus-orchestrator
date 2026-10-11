import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Link } from 'react-router';
import { isApiError } from '../../api/errors';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import type { Script, ScriptsApi } from './api';
import { lineDiff } from './diff';
import type { LineDiff } from './diff';
import { FIELD_LABEL } from './draft';
import type { DraftField, DraftSaver, DraftSnapshot } from './draft';
import { checkSchemaText, sameSchema, schemaToText } from './jsonText';
import { SCRIPT_NAME } from './languages';
import { keys } from './queries';
import { clipLabel, formatTime } from './text';

const FIELDS: readonly DraftField[] = ['source', 'inputSchema', 'outputSchema'];

function DiffView({ diff, label }: { diff: LineDiff; label: string }) {
  if (diff.added === 0 && diff.removed === 0) return <p>No difference in {label}.</p>;
  return (
    <>
      <p>
        {label.charAt(0).toUpperCase()}
        {label.slice(1)}: {diff.added} {diff.added === 1 ? 'line' : 'lines'} only in yours, {diff.removed} only in theirs.
        {diff.exact ? '' : ' The two are too different to compare line by line; theirs is shown first, then yours.'}
      </p>
      {/* A diff can be long; the region scrolls and so must be reachable by keyboard. */}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex */}
      <pre className="diff" role="region" aria-label={`Differences in ${label}`} tabIndex={0}>
        {diff.lines.map((line, index) =>
          line.kind === 'gap' ? (
            <span key={index} className="diff-gap">
              {`… ${line.hidden} unchanged ${line.hidden === 1 ? 'line' : 'lines'} …\n`}
            </span>
          ) : (
            <span key={index} className={`diff-${line.kind}`}>
              <span className="visually-hidden">{line.kind === 'added' ? 'Yours: ' : line.kind === 'removed' ? 'Theirs: ' : 'Both: '}</span>
              {`${line.kind === 'added' ? '+ ' : line.kind === 'removed' ? '- ' : '  '}${line.text}\n`}
            </span>
          ),
        )}
      </pre>
    </>
  );
}

/**
 * Two people changed the same draft (a `409` on the revision). Nothing is overwritten on either
 * side until the author has seen what differs and chosen: take the platform's copy, keep theirs
 * changes over it, or save their version as a script of its own (D146 of feature 007).
 */
export function ConflictPanel({ api, scriptId, scriptName, language, saver, draft }: { api: ScriptsApi; scriptId: string; scriptName: string; language: string; saver: DraftSaver; draft: DraftSnapshot }) {
  const announce = useAnnounce();
  const queries = useQueryClient();
  const [theirs, setTheirs] = useState<Script | undefined>(undefined);
  const [error, setError] = useState<unknown>(undefined);
  const [busy, setBusy] = useState(false);
  const [copying, setCopying] = useState(false);
  const [copied, setCopied] = useState<Script | undefined>(undefined);
  const heading = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    heading.current?.focus();
    const abort = new AbortController();
    api.get(scriptId, abort.signal).then(setTheirs, (failure: unknown) => {
      if (!abort.signal.aborted) setError(failure);
    });
    return () => abort.abort();
  }, [api, scriptId]);

  const theirText = useMemo(() => (theirs ? { source: theirs.source, inputSchema: schemaToText(theirs.inputSchema), outputSchema: schemaToText(theirs.outputSchema) } : undefined), [theirs]);
  const diffs = useMemo(() => {
    if (!theirText) return [];
    return FIELDS.map((field) => {
      // A schema is compared as a schema: the author's layout is not a difference.
      const same = field !== 'source' && sameSchema(theirText[field], draft.fields[field]);
      return { field, diff: lineDiff(theirText[field], same ? theirText[field] : draft.fields[field]) };
    });
  }, [theirText, draft.fields]);
  const current = draft.state.kind === 'conflict' ? draft.state.currentRevision : undefined;

  const takeTheirs = () => {
    if (!theirs) return;
    saver.adoptTheirs(theirs);
    void queries.invalidateQueries({ queryKey: keys.script(api.namespaceId, scriptId) });
    announce('Loaded the platform’s copy of the draft. Your changes were dropped.');
  };
  const keepMine = () => {
    if (!theirs) return;
    setBusy(true);
    saver.keepMine(theirs);
    void saver.save().then((ok) => {
      setBusy(false);
      announce(ok ? 'Your changes were saved over the other copy.' : 'Your changes could not be saved.');
    });
  };

  return (
    <section className="conflict" aria-labelledby="conflict-title">
      <h2 id="conflict-title" ref={heading} tabIndex={-1}>
        Someone else changed this draft
      </h2>
      <Banner tone="warn">
        Your changes are kept here and are <strong>not saved</strong>: the platform&apos;s draft is at revision {theirs?.revision ?? current ?? 'newer than yours'}, and yours is based on revision {draft.revision}. Nothing is overwritten until you choose.
      </Banner>
      {error !== undefined ? <ErrorState error={error} what="Reading the platform’s copy" /> : null}
      {theirs === undefined && error === undefined ? <p role="status">Reading the platform&apos;s copy…</p> : null}
      {theirs ? (
        <>
          <p>
            The platform&apos;s copy was last changed <time dateTime={theirs.updatedAt}>{formatTime(theirs.updatedAt)}</time>. Lines marked + are only in your copy, lines marked - only in theirs.
          </p>
          {diffs.map(({ field, diff }) => (
            <DiffView key={field} diff={diff} label={FIELD_LABEL[field].toLowerCase()} />
          ))}
          <div className="toolbar">
            <Button onClick={takeTheirs} disabled={busy}>
              Load theirs (drop my changes)
            </Button>
            <Button variant="danger" onClick={keepMine} disabled={busy}>
              {busy ? 'Saving…' : 'Keep mine and save over theirs'}
            </Button>
            <Button onClick={() => setCopying(true)} disabled={busy}>
              Save mine as a new script…
            </Button>
          </div>
          {copied ? (
            <Banner tone="ok">
              Saved as <Link to={`/scripts/${encodeURIComponent(copied.id)}`}>{clipLabel(copied.name)}</Link>. This draft still has the conflict: load theirs or keep yours.
            </Banner>
          ) : null}
        </>
      ) : null}
      <CopyDialog
        open={copying}
        api={api}
        language={language}
        suggestion={`${scriptName}-copy`}
        draft={draft}
        onClose={() => setCopying(false)}
        onCopied={(script) => {
          setCopying(false);
          setCopied(script);
          announce(`Saved your version as the new script ${script.name}.`);
        }}
      />
    </section>
  );
}

function CopyDialog({ open, api, language, suggestion, draft, onClose, onCopied }: { open: boolean; api: ScriptsApi; language: string; suggestion: string; draft: DraftSnapshot; onClose: () => void; onCopied: (script: Script) => void }) {
  return (
    <Dialog open={open} title="Save my version as a new script" onClose={onClose}>
      <CopyForm api={api} language={language} suggestion={suggestion} draft={draft} onClose={onClose} onCopied={onCopied} />
    </Dialog>
  );
}

function CopyForm({ api, language, suggestion, draft, onClose, onCopied }: { api: ScriptsApi; language: string; suggestion: string; draft: DraftSnapshot; onClose: () => void; onCopied: (script: Script) => void }) {
  const queries = useQueryClient();
  const [name, setName] = useState(suggestion.slice(0, 63));
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<unknown>(undefined);
  const nameError = isApiError(problem) && problem.status === 409 ? 'A script with this name already exists in this namespace.' : name !== '' && !SCRIPT_NAME.test(name) ? 'Use lower-case letters, digits and hyphens, starting with a letter.' : undefined;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy || !SCRIPT_NAME.test(name)) return;
    setBusy(true);
    setProblem(undefined);
    const input = checkSchemaText(draft.fields.inputSchema);
    const output = checkSchemaText(draft.fields.outputSchema);
    api
      .create({
        name,
        language,
        source: draft.fields.source,
        ...(input.kind === 'ok' ? { inputSchema: input.value } : {}),
        ...(output.kind === 'ok' ? { outputSchema: output.value } : {}),
      })
      .then(
        (script) => {
          void queries.invalidateQueries({ queryKey: keys.list(api.namespaceId) });
          onCopied(script);
        },
        (failure: unknown) => {
          setBusy(false);
          setProblem(failure);
        },
      );
  };
  return (
    <form onSubmit={submit} noValidate>
      <p>Your copy of the source and the schemas becomes a new script in this namespace. The script you are editing is not changed.</p>
      <Field label="Name" error={nameError}>
        <input value={name} onChange={(event) => setName(event.target.value)} autoComplete="off" spellCheck={false} maxLength={63} required />
      </Field>
      {problem !== undefined && !(isApiError(problem) && problem.status === 409) ? <ErrorState error={problem} what="Saving the copy" /> : null}
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={busy || !SCRIPT_NAME.test(name)}>
          {busy ? 'Saving…' : 'Save as a new script'}
        </Button>
      </div>
    </form>
  );
}
