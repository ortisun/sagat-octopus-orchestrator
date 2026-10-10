import { useId, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import type { Operation } from './ops';

/**
 * The operations on something, one button each. One that cannot be asked for now stays on the
 * page and in the tab order (`aria-disabled`, not `disabled`) with the reason next to it in
 * text, so nobody has to guess why it is not there.
 */
export function OperationList({ operations, onChoose, label }: { operations: readonly Operation[]; onChoose: (operation: Operation, opener: HTMLElement | null) => void; label: string }) {
  const base = useId();
  return (
    <ul className="rn-operations" aria-label={label}>
      {operations.map((operation) => {
        const reasonId = `${base}-${operation.action}`;
        return (
          <li key={operation.action} data-action={operation.action}>
            <button
              type="button"
              className="btn"
              aria-disabled={operation.enabled ? undefined : true}
              aria-describedby={operation.enabled ? undefined : reasonId}
              onClick={(event) => {
                if (operation.enabled) onChoose(operation, event.currentTarget);
              }}
            >
              {operation.label}
            </button>
            {operation.enabled ? null : (
              <span id={reasonId} className="rn-reason">
                {operation.reason}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * A button that shows the operations when pressed: a disclosure (`aria-expanded`), operable with
 * Enter or Space, closed with Escape, which returns focus to the button. `inline` opens the list
 * in the flow of the page (inside a table row); otherwise it floats under the button.
 */
export function ActionsMenu({
  label,
  buttonText,
  operations,
  onChoose,
  inline = false,
}: {
  label: string;
  buttonText: string;
  operations: readonly Operation[];
  onChoose: (operation: Operation, opener: HTMLElement | null) => void;
  inline?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && open) {
      event.stopPropagation();
      setOpen(false);
      trigger.current?.focus();
    }
  };
  return (
    <div
      role="presentation"
      className={inline ? 'rn-menu inline' : 'rn-menu'}
      onKeyDown={onKeyDown}
      onBlur={(event) => {
        // Focus left the menu altogether: close it (not when it moves from the button to an item).
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button ref={trigger} type="button" className="btn" aria-expanded={open} aria-controls={open ? listId : undefined} aria-label={label} onClick={() => setOpen((value) => !value)}>
        {buttonText} <span aria-hidden="true">{open ? '▴' : '▾'}</span>
      </button>
      {open ? (
        // Safari does not focus a button when it is pressed: without this the menu would close on the press and lose the click.
        <div id={listId} role="presentation" className="rn-menu-list" onMouseDown={(event) => event.preventDefault()}>
          <OperationList
            operations={operations}
            label={label}
            onChoose={(operation) => {
              setOpen(false);
              onChoose(operation, trigger.current);
            }}
          />
        </div>
      ) : null}
    </div>
  );
}
