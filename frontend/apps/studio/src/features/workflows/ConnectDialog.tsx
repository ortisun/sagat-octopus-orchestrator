import { useState } from 'react';
import type { IrDocument } from '../../model/ir';
import { kindOf } from '../../model/registry';
import { Button } from '../../ui/Button';
import { Field } from '../../ui/Field';
import { clip } from './canvasModel';
import { Modal } from './Modal';

function ConnectBody({ doc, from, onClose, onConnect }: { doc: IrDocument; from: string; onClose: () => void; onConnect: (field: string, to: string) => string | undefined }) {
  const node = doc.nodes.find((n) => n.id === from);
  const handles = kindOf(node?.type ?? '')?.handles ?? [];
  const [field, setField] = useState(handles[0]?.field ?? '');
  const [to, setTo] = useState('');
  const [error, setError] = useState<string | undefined>();
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const reason = onConnect(field, to);
        setError(reason);
        if (reason === undefined) onClose();
      }}
    >
      <p>
        Make <code>{clip(from, 64)}</code> lead to another node. A link that was there is replaced; a list such as branches gets one more.
      </p>
      <Field label="Link">
        <select value={field} onChange={(event) => setField(event.target.value)}>
          {handles.map((h) => (
            <option key={h.field} value={h.field}>
              {h.label}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Leads to" error={error}>
        <select value={to} onChange={(event) => setTo(event.target.value)}>
          <option value="">(choose a node)</option>
          {doc.nodes
            .filter((n) => n.id !== from)
            .map((n) => (
              <option key={n.id} value={n.id}>
                {clip(n.id, 64)} ({kindOf(n.type)?.label ?? n.type})
              </option>
            ))}
        </select>
      </Field>
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={to === '' || field === ''}>
          Connect
        </Button>
      </div>
    </form>
  );
}

/** The list view's way to draw a link: pick the way out and the node it leads to. */
export function ConnectDialog({
  open,
  doc,
  from,
  onClose,
  onConnect,
}: {
  open: boolean;
  doc: IrDocument;
  from: string | undefined;
  onClose: () => void;
  onConnect: (field: string, to: string) => string | undefined;
}) {
  return (
    <Modal open={open && from !== undefined} title="Connect…" onClose={onClose}>
      {from !== undefined ? <ConnectBody doc={doc} from={from} onClose={onClose} onConnect={onConnect} /> : null}
    </Modal>
  );
}
