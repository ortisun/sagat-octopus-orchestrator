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
  // What had focus when the dialog opened: the control to give it back to when it closes. The browser does this itself on `close()`;
  // saying it here as well makes it the behaviour of this component, the same in every browser and in a test, and it is a no-op where
  // the browser has done it. A control that is gone by then has nothing to give it back to, and the owner decides where focus goes.
  const returnTo = useRef<Element | null>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      returnTo.current = document.activeElement;
      if (typeof dialog.showModal === 'function') dialog.showModal();
      else dialog.setAttribute('open', '');
    } else if (!open && dialog.open) {
      dialog.close();
      const target = returnTo.current;
      returnTo.current = null;
      if (target instanceof HTMLElement && target !== document.body && target.isConnected) target.focus();
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
      // closed it through `open`, there is nothing to tell. The browser fires `close` in a task of its own, after `close()`: a person who
      // pressed Escape and then Enter on the opener can have reopened the dialog by then, and that event is the old closing's, not a
      // closing of this one. It counts only if the element is closed now.
      onClose={(event) => {
        if (open && !event.currentTarget.open) onClose();
      }}
    >
      <h2 id={titleId}>{title}</h2>
      {open ? children : null}
    </dialog>
  );
}
