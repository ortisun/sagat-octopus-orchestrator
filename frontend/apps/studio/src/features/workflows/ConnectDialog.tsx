import { useState } from 'react';
import type { IrDocument } from '../../model/ir';
import { kindOf } from '../../model/registry';
import { Actions, Button, Code, Select, Stack, Text } from '../../ui';
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
      <Stack gap={3}>
        <Text>
          Make <Code>{clip(from, 64)}</Code> lead to another node. A link that was there is replaced; a list such as branches gets one more.
        </Text>
        <Select label="Link" value={field} options={handles.map((h) => ({ value: h.field, label: h.label }))} onChange={setField} />
        <Select
          label="Leads to"
          error={error}
          value={to}
          placeholder="(choose a node)"
          options={doc.nodes.filter((n) => n.id !== from).map((n) => ({ value: n.id, label: `${clip(n.id, 64)} (${kindOf(n.type)?.label ?? n.type})` }))}
          onChange={setTo}
        />
        <Actions>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={to === '' || field === ''}>
            Connect
          </Button>
        </Actions>
      </Stack>
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
