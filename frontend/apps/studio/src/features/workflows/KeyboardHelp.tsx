import { Button } from '../../ui/Button';
import { Modal } from './Modal';

const SHORTCUTS: readonly { keys: string; does: string }[] = [
  { keys: '?', does: 'Open this help (when you are not typing in a field).' },
  { keys: 'Alt + N', does: 'Add a node: opens the menu of kinds. The node goes after the selected node, or to the middle of the canvas.' },
  { keys: 'Ctrl/Cmd + Z', does: 'Undo the last change to the workflow (when you are not typing in a field).' },
  { keys: 'Ctrl/Cmd + Shift + Z, or Ctrl + Y', does: 'Redo.' },
  { keys: 'Ctrl/Cmd + S', does: 'Save the draft now.' },
  { keys: 'Tab', does: 'Move into the canvas (to the card last visited) and out of it.' },
  { keys: 'Arrow keys', does: 'On a card: along the flow follows the first link out, against it goes back, across it steps between the nodes one node leads to.' },
  { keys: 'Enter', does: 'On a card: open its inspector. Space selects it.' },
  { keys: 'Delete or Backspace', does: 'On a card: delete the node and the links to it. Undo brings it back.' },
];

export function KeyboardHelp({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Modal open={open} title="Keyboard help" onClose={onClose}>
      <p>Everything the canvas does can also be done in the list view, from the inspector and from the toolbar.</p>
      <table className="data">
        <caption className="visually-hidden">Keyboard shortcuts of the workflow editor</caption>
        <thead>
          <tr>
            <th scope="col">Keys</th>
            <th scope="col">What they do</th>
          </tr>
        </thead>
        <tbody>
          {SHORTCUTS.map((s) => (
            <tr key={s.keys}>
              <th scope="row">
                <kbd>{s.keys}</kbd>
              </th>
              <td>{s.does}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="dialog-actions">
        <Button onClick={onClose} variant="primary">
          Close
        </Button>
      </div>
    </Modal>
  );
}
