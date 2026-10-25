import { useId } from 'react';
import type { DragEvent } from 'react';
import { NODE_KINDS } from '../../model/registry';
import type { NodeKindSpec } from '../../model/registry';
import { Button } from '../../ui/Button';
import { PALETTE_MIME } from './canvasModel';
import { Modal } from './Modal';

function KindButton({ kind, onPick, draggable }: { kind: NodeKindSpec; onPick: (type: string) => void; draggable: boolean }) {
  const labelId = useId();
  const descId = useId();
  const onDragStart = (event: DragEvent<HTMLButtonElement>) => {
    event.dataTransfer.setData(PALETTE_MIME, kind.type);
    event.dataTransfer.effectAllowed = 'copy';
  };
  return (
    <button
      type="button"
      className="wf-kind"
      data-category={kind.category}
      draggable={draggable}
      onDragStart={draggable ? onDragStart : undefined}
      aria-labelledby={labelId}
      aria-describedby={descId}
      onClick={() => onPick(kind.type)}
    >
      <span id={labelId} className="wf-kind-label">
        {kind.label}
      </span>
      <span id={descId} className="wf-kind-desc">
        {kind.description}
      </span>
    </button>
  );
}

/** Every kind of node, to drag onto the canvas or to press: both add the same node. */
export function Palette({ onPick }: { onPick: (type: string) => void }) {
  return (
    <section className="wf-palette" aria-label="Palette">
      <h2 className="wf-panel-title">Add a node</h2>
      <p className="hint">Drag a kind onto the canvas, or press it to add it where the dialog &quot;Add node&quot; would.</p>
      <ul>
        {NODE_KINDS.map((kind) => (
          <li key={kind.type}>
            <KindButton kind={kind} onPick={onPick} draggable />
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The keyboard way to add a node: a menu of the kinds, opened by the button or Alt+N. */
export function AddNodeDialog({ open, where, onClose, onPick, kinds = NODE_KINDS }: { open: boolean; where: string; onClose: () => void; onPick: (type: string) => void; kinds?: readonly NodeKindSpec[] }) {
  return (
    <Modal open={open} title="Add a node" onClose={onClose}>
      <p>{where}</p>
      <ul className="wf-kinds">
        {kinds.map((kind) => (
          <li key={kind.type}>
            <KindButton
              kind={kind}
              draggable={false}
              onPick={(type) => {
                onClose();
                onPick(type);
              }}
            />
          </li>
        ))}
      </ul>
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
      </div>
    </Modal>
  );
}
