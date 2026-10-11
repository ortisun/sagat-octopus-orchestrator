import { useId, useMemo, useRef, useState } from 'react';
import { isApiError } from '../../api/errors';
import { Banner } from '../../ui/Banner';
import type { SaveState } from './draft';
import { CodeEditor } from './CodeEditor';
import type { EditorControls } from './CodeEditor';
import { checkSchemaText } from './jsonText';
import type { Problem } from './monacoSetup';
import { ProblemsList } from './ProblemsList';

const META = { kind: 'meta' } as const;

function SchemaEditor({
  title,
  field,
  value,
  readOnly,
  onChange,
}: {
  title: string;
  field: 'inputSchema' | 'outputSchema';
  value: string;
  readOnly: boolean;
  onChange: (text: string) => void;
}) {
  const id = useId();
  const controls = useRef<EditorControls | null>(null);
  const [problems, setProblems] = useState<Problem[]>([]);
  const check = useMemo(() => checkSchemaText(value), [value]);
  return (
    <section className="schema-editor" aria-labelledby={`${id}-title`}>
      <h3 id={`${id}-title`}>{title}</h3>
      <CodeEditor label={`${title} (JSON)`} describedBy={`${id}-state`} model={field} language="json" value={value} onChange={onChange} readOnly={readOnly} jsonSchema={META} onProblems={setProblems} controlsRef={controls} height="14rem" />
      <p id={`${id}-state`} className={check.kind === 'invalid' ? 'error' : 'hint'}>
        {check.kind === 'empty' ? 'No schema: nothing is checked.' : check.kind === 'ok' ? 'A JSON object. The platform compiles it when the draft is saved.' : check.message}
      </p>
      <ProblemsList problems={problems} controlsRef={controls} label={id} />
    </section>
  );
}

/** The input and output JSON Schemas of the script: written here, checked in the browser, compiled by the platform when saved. */
export function SchemasPanel({
  inputSchema,
  outputSchema,
  readOnly,
  onChange,
  saveState,
}: {
  inputSchema: string;
  outputSchema: string;
  readOnly: boolean;
  onChange: (field: 'inputSchema' | 'outputSchema', text: string) => void;
  saveState: SaveState;
}) {
  const refused = saveState.kind === 'rejected' && isApiError(saveState.error) && saveState.error.reason === 'SCHEMA';
  return (
    <div className="schemas-panel">
      <p>
        The input is checked against the input schema before the script runs, and the result against the output schema after. They become part of the published version. Underlines come from the JSON Schema draft-07 meta-schema, bundled with the Studio; the platform has the last word when the draft is saved (a schema must not refer outside itself, and a pattern that needs lookaround or backreferences is refused).
      </p>
      {refused && saveState.kind === 'rejected' && isApiError(saveState.error) ? (
        <Banner tone="error" title="The platform did not accept a schema.">
          {saveState.error.message} It does not say which one: check both.
        </Banner>
      ) : null}
      <SchemaEditor title="Input schema" field="inputSchema" value={inputSchema} readOnly={readOnly} onChange={(text) => onChange('inputSchema', text)} />
      <SchemaEditor title="Output schema" field="outputSchema" value={outputSchema} readOnly={readOnly} onChange={(text) => onChange('outputSchema', text)} />
    </div>
  );
}
