import { useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';

/**
 * A modal dialog on the platform's own `<dialog>`: it traps focus, closes on Escape, makes the page
 * behind it inert and returns focus to what opened it, which is the behaviour people with assistive
 * technology expect and the one a hand-made dialog gets wrong.
 */
export function Dialog({ open, title, onClose, children }: { open: boolean; title: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      if (typeof dialog.showModal === 'function') dialog.showModal();
      else dialog.setAttribute('open', '');
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);
  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      // Escape: the owner decides (it sets `open` to false and the effect closes the dialog), so the
      // browser must not close it behind the owner's back.
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      // The dialog closed itself (a form with method="dialog"): tell the owner, once. When the owner
      // closed it through `open`, there is nothing to tell.
      onClose={() => {
        if (open) onClose();
      }}
    >
      <h2 id={titleId}>{title}</h2>
      {open ? children : null}
    </dialog>
  );
}
