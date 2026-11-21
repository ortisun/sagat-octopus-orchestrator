import { forwardRef, useState } from 'react';
import type { Ref } from 'react';
import { connect, disconnect } from '../../model/edit';
import type { IrDocument, IrNode } from '../../model/ir';
import { kindOf } from '../../model/registry';
import type { HandleSpec } from '../../model/registry';
import { Banner, Button, Code, FocusHeading, Heading, Hint, Row, Select, Stack, TextInput } from '../../ui';
import { FieldEditor, Group } from './fields';
import type { Change } from './fields';
import { DatabaseFields } from './DatabaseFields';
import { ScriptPicker } from './ScriptPicker';

export interface NodeNote {
  kind: 'problem' | 'warning' | 'advice';
  text: string;
}

export interface InspectorActions {
  change: Change;
  /** Returns the reason it was refused, or nothing. */
  rename: (from: string, to: string) => string | undefined;
  setFirst: (id: string) => void;
  remove: (id: string) => void;
}

const NOTE_LABEL: Record<NodeNote['kind'], string> = { problem: 'Problem', warning: 'Warning', advice: 'Advice' };
const NOTE_SHAPE: Record<NodeNote['kind'], string> = { problem: '✕', warning: '!', advice: '·' };

function RenameField({ node, disabled, rename }: { node: IrNode; disabled: boolean; rename: InspectorActions['rename'] }) {
  const [draft, setDraft] = useState<{ id: string; text: string } | undefined>();
  const [error, setError] = useState<string | undefined>();
  const text = draft?.id === node.id ? draft.text : node.id;
  const apply = () => {
    if (text === node.id) return;
    const reason = rename(node.id, text);
    setError(reason);
    if (reason === undefined) setDraft(undefined);
  };
  return (
    <div className="wf-rename">
      <TextInput
        label="Id"
        hint="How other nodes and the run refer to this node."
        error={error}
        value={text}
        disabled={disabled}
        autoComplete="off"
        mono
        onChange={(value) => {
          setDraft({ id: node.id, text: value });
          setError(undefined);
        }}
        onBlur={apply}
        onEnter={apply}
      />
      {disabled ? null : (
        <Button onClick={apply} disabled={text === node.id}>
          Rename
        </Button>
      )}
    </div>
  );
}

function optionLabel(doc: IrDocument, id: string): string {
  const node = doc.nodes.find((n) => n.id === id);
  return node ? `${id} (${kindOf(node.type)?.label ?? node.type})` : id;
}

/** The ids a link can point at: every node but this one. */
function targets(doc: IrDocument, selfId: string): string[] {
  return doc.nodes.filter((n) => n.id !== selfId).map((n) => n.id);
}

function SingleLink({ doc, node, handle, disabled, change }: { doc: IrDocument; node: IrNode; handle: HandleSpec; disabled: boolean; change: Change }) {
  const value = typeof node[handle.field] === 'string' ? (node[handle.field] as string) : '';
  const ids = targets(doc, node.id);
  const missing = value !== '' && !ids.includes(value);
  return (
    <Select
      label={handle.label}
      value={value}
      placeholder="—"
      disabled={disabled}
      options={[
        { value: '', label: '—' },
        ...(missing ? [{ value, label: `${value} (no such node)` }] : []),
        ...ids.map((id) => ({ value: id, label: optionLabel(doc, id) })),
      ]}
      onChange={(to) => change((d) => (to === '' ? disconnect(d, node.id, handle.field) : connect(d, node.id, handle.field, to)), `link:${node.id}:${handle.field}`)}
    />
  );
}

function ListLink({ doc, node, handle, disabled, change }: { doc: IrDocument; node: IrNode; handle: HandleSpec; disabled: boolean; change: Change }) {
  const raw = node[handle.field];
  const list = Array.isArray(raw) ? raw.filter((t): t is string => typeof t === 'string') : [];
  const [adding, setAdding] = useState('');
  const ids = targets(doc, node.id);
  return (
    <Group label={`${handle.label}es`}>
      {list.length === 0 ? <Hint>None yet.</Hint> : null}
      <ol className="wf-branches">
        {list.map((target, index) => (
          <li key={`${target}-${index}`}>
            <span>
              {handle.label} {index + 1}: <Code>{target}</Code>
            </span>
            {disabled ? null : (
              <Button
                aria-label={`Remove ${handle.label.toLowerCase()} ${index + 1}, ${target}`}
                onClick={() => change((d) => disconnect(d, node.id, handle.field, index), `link:${node.id}:${handle.field}`)}
              >
                Remove
              </Button>
            )}
          </li>
        ))}
      </ol>
      {disabled ? null : (
        <div className="wf-add-row">
          <Select
            label={`Add a ${handle.label.toLowerCase()}`}
            hideLabel
            value={adding}
            placeholder="(choose a node)"
            options={ids.map((id) => ({ value: id, label: optionLabel(doc, id) }))}
            onChange={setAdding}
          />
          <Button
            disabled={adding === ''}
            onClick={() => {
              change((d) => connect(d, node.id, handle.field, adding), `link:${node.id}:${handle.field}`);
              setAdding('');
            }}
          >
            Add {handle.label.toLowerCase()}
          </Button>
        </div>
      )}
    </Group>
  );
}

/** The inspector of the selected node: its id, its fields, where it leads, and what is wrong with it. */
export const NodeInspector = forwardRef(function NodeInspector(
  { doc, node, disabled, notes, actions }: { doc: IrDocument; node: IrNode; disabled: boolean; notes: readonly NodeNote[]; actions: InspectorActions },
  ref: Ref<HTMLHeadingElement>,
) {
  const spec = kindOf(node.type);
  const first = doc.entrypoint === node.id;
  return (
    <Stack gap={3}>
      <FocusHeading level={2} look={3} ref={ref}>
        {spec?.label ?? node.type}: {node.id}
      </FocusHeading>
      {spec ? (
        <Hint>{spec.description}</Hint>
      ) : (
        <Banner tone="warn">This kind of node ({node.type}) is not one this Studio knows. It is kept as it is and cannot be edited here.</Banner>
      )}
      {spec && !spec.runs ? <Banner tone="warn">The platform cannot run this node yet; you can draw it but not publish it.</Banner> : null}

      <RenameField node={node} disabled={disabled} rename={actions.rename} />

      {node.type === 'database' ? <DatabaseFields node={node} disabled={disabled} change={actions.change} /> : null}

      {(spec?.fields ?? []).map((field) => (
        <Stack key={field.name} gap={3}>
          {node.type === 'script' && field.name === 'artifactDigest' ? <ScriptPicker node={node} disabled={disabled} change={actions.change} /> : null}
          <FieldEditor spec={field} node={node} disabled={disabled} change={actions.change} />
        </Stack>
      ))}

      {(spec?.handles ?? []).length > 0 ? (
        <Stack as="section" gap={3} aria-label="Where this node leads">
          <Heading level={3} look={4}>
            Leads to
          </Heading>
          {(spec?.handles ?? []).map((handle) =>
            handle.multiple ? (
              <ListLink key={handle.field} doc={doc} node={node} handle={handle} disabled={disabled} change={actions.change} />
            ) : (
              <SingleLink key={handle.field} doc={doc} node={node} handle={handle} disabled={disabled} change={actions.change} />
            ),
          )}
        </Stack>
      ) : null}

      {disabled ? null : (
        <Row>
          <Button onClick={() => actions.setFirst(node.id)} disabled={first} aria-describedby={first ? 'wf-first-note' : undefined}>
            Set as first step
          </Button>
          {first ? <Hint id="wf-first-note">This is the first step.</Hint> : null}
          <Button variant="danger" onClick={() => actions.remove(node.id)}>
            Delete node
          </Button>
        </Row>
      )}

      {notes.length > 0 ? (
        <Stack as="section" gap={2} aria-label="Checks for this node">
          <Heading level={3} look={4}>
            Checks for this node
          </Heading>
          <ul className="wf-notes">
            {notes.map((note, index) => (
              <li key={index} data-kind={note.kind}>
                <span aria-hidden="true">{NOTE_SHAPE[note.kind]}</span> <strong>{NOTE_LABEL[note.kind]}.</strong> {note.text}
              </li>
            ))}
          </ul>
        </Stack>
      ) : null}
    </Stack>
  );
});
