import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import type { FormEvent } from 'react';
import { useNavigate } from 'react-router';
import { isApiError } from '../../api/errors';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';
import type { ScriptsApi } from './api';
import { LANGUAGES, NOT_BUILT, SCRIPT_NAME, sampleSource } from './languages';

/** The problem a refusal of `createScript` is about, for the field it belongs to. */
function refusal(error: unknown): { name?: string; language?: string; other?: unknown } {
  if (!isApiError(error)) return { other: error };
  if (error.status === 409 && error.details?.['field'] === 'name') return { name: 'A script with this name already exists in this namespace.' };
  if (error.reason === 'LANGUAGE_NOT_BUILT') return { language: `This language is ${NOT_BUILT}: only TypeScript and Python can be built and run today.` };
  if (error.status === 422 && error.reason === 'IDENTIFIER') return { name: error.message };
  if (error.status === 422 && error.message.toLowerCase().includes('slug')) return { name: error.message };
  return { other: error };
}

/** Make a script: a name, a language, and the sample for that language to start from. */
export function CreateScriptDialog({ open, api, onClose }: { open: boolean; api: ScriptsApi; onClose: () => void }) {
  return (
    <Dialog open={open} title="New script" onClose={onClose}>
      <CreateForm api={api} onClose={onClose} />
    </Dialog>
  );
}

function CreateForm({ api, onClose }: { api: ScriptsApi; onClose: () => void }) {
  const queries = useQueryClient();
  const navigate = useNavigate();
  const announce = useAnnounce();
  const [name, setName] = useState('');
  const [language, setLanguage] = useState('typescript');
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<ReturnType<typeof refusal>>({});
  const nameInvalid = name !== '' && !SCRIPT_NAME.test(name);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy || !SCRIPT_NAME.test(name)) return;
    setBusy(true);
    setProblem({});
    api.create({ name, language, source: sampleSource(language) }).then(
      (script) => {
        void queries.invalidateQueries({ queryKey: ['scripts', api.namespaceId] });
        announce(`Script ${script.name} created.`);
        onClose();
        void navigate(`/scripts/${encodeURIComponent(script.id)}`);
      },
      (error: unknown) => {
        setBusy(false);
        setProblem(refusal(error));
      },
    );
  };

  return (
    <form onSubmit={submit} noValidate>
      <Field label="Name" hint="3 to 63 characters: lower-case letters, digits and hyphens, starting with a letter." error={problem.name ?? (nameInvalid ? 'Use lower-case letters, digits and hyphens, starting with a letter and not ending with a hyphen.' : undefined)}>
        <input value={name} onChange={(event) => setName(event.target.value)} autoComplete="off" spellCheck={false} maxLength={63} required />
      </Field>
      <Field label="Language" hint="The platform builds and runs TypeScript and Python today." error={problem.language}>
        <select value={language} onChange={(event) => setLanguage(event.target.value)}>
          {LANGUAGES.map((option) => (
            <option key={option.id} value={option.id} disabled={!option.built}>
              {option.built ? option.label : `${option.label} (${NOT_BUILT})`}
            </option>
          ))}
        </select>
      </Field>
      <p className="hint">The script starts from a small sample for the language; you can change everything.</p>
      {problem.other !== undefined ? <ErrorState error={problem.other} what="Creating the script" /> : null}
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={busy || !SCRIPT_NAME.test(name)}>
          {busy ? 'Creating…' : 'Create script'}
        </Button>
      </div>
    </form>
  );
}
