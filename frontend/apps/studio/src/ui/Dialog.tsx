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
    <dialog ref={ref} aria-labelledby={titleId} onClose={onClose} onCancel={onClose}>
      <h2 id={titleId}>{title}</h2>
      {open ? children : null}
    </dialog>
  );
}
