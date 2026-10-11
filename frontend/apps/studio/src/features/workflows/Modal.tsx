import { useLayoutEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { Dialog } from '../../ui/Dialog';

/**
 * The shared dialog, plus the one thing the editor needs from it everywhere: when it closes, focus
 * goes back to what opened it. A browser does that for `<dialog>`; doing it here too makes it hold
 * where the opener was rebuilt meanwhile and in the test environment, and costs nothing otherwise.
 */
export function Modal({ open, title, onClose, children }: { open: boolean; title: string; onClose: () => void; children: ReactNode }) {
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (open) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      return;
    }
    const element = opener.current;
    opener.current = null;
    if (element?.isConnected === true) element.focus();
  }, [open]);
  return (
    <Dialog open={open} title={title} onClose={onClose}>
      {children}
    </Dialog>
  );
}
