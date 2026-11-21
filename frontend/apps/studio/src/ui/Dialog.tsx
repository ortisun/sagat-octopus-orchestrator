import { Button as AstryxButton } from '@astryxdesign/core/Button';
import { Dialog as AstryxDialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { HStack } from '@astryxdesign/core/HStack';
import { VStack } from '@astryxdesign/core/VStack';
import { useEffect, useEffectEvent, useLayoutEffect, useRef } from 'react';
import type { ReactNode } from 'react';

/**
 * A modal dialog. Astryx draws it on the platform's own `<dialog>`: it traps focus, closes on Escape,
 * makes the page behind it inert and returns focus to what opened it. The owner decides when it is
 * open; `onClose` is the request to close it (Escape, or the close button in its header). On opening,
 * focus goes to the first control of the body that can be used, as the platform's dialog does; the
 * title takes it only when the body has none.
 */
export function Dialog({ open, title, onClose, children, width }: { open: boolean; title: string; onClose: () => void; children: ReactNode; width?: number | string }) {
  const element = useRef<HTMLDialogElement>(null);
  // Reads the owner's `open` and `onClose` as they are now, not as they were when the listener was added.
  const closedByItself = useEffectEvent((dialog: HTMLDialogElement) => {
    if (open && !dialog.open) onClose();
  });
  // The browser can close the element by itself (a form with method="dialog"), which the design system
  // does not report: tell the owner, once. The `close` event comes in a task of its own, so one fired
  // by an earlier closing can arrive after the dialog was opened again; it counts only if the element
  // is closed now.
  useEffect(() => {
    const dialog = element.current;
    if (!dialog) return;
    const closed = () => closedByItself(dialog);
    dialog.addEventListener('close', closed);
    return () => dialog.removeEventListener('close', closed);
  }, []);
  return (
    <AstryxDialog
      ref={element}
      isOpen={open}
      purpose="form"
      {...(width !== undefined ? { width } : { width: 560 })}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogHeader
        title={title}
        // A way out that does not depend on the body: some bodies offer no button of their own. Its name
        // says what it closes and does not repeat the title, which a button of the body may also say.
        endContent={
          <AstryxButton variant="ghost" size="sm" label="Close dialog" onClick={onClose}>
            <span aria-hidden="true">✕</span>
          </AstryxButton>
        }
      />
      {open ? (
        <VStack gap={3} padding={4}>
          {children}
          <FirstControl />
        </VStack>
      ) : null}
    </AstryxDialog>
  );
}

const USABLE = ['input:not([type="hidden"])', 'textarea', 'button', '[role="combobox"]', 'a[href]', '[tabindex]:not([tabindex="-1"])']
  .map((selector) => `${selector}:not([disabled]):not([aria-disabled="true"])`)
  .join(', ');

/**
 * Marks the first control of the body that can be used as the one to focus when the dialog opens. The
 * design system focuses the title unless a descendant asks for focus with `data-autofocus`. The mark is
 * set in a layout effect, before the design system opens the dialog; the body is found from an element
 * of this component's own, because React attaches the body's ref only after its children's layout effects.
 */
function FirstControl() {
  const probe = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    probe.current?.parentElement?.querySelector(USABLE)?.setAttribute('data-autofocus', 'first-control');
  }, []);
  return <span ref={probe} hidden />;
}

/** The row of a dialog's or a form's actions: the safe one first, the one that commits last. */
export function Actions({ children }: { children: ReactNode }) {
  return (
    <HStack gap={2} hAlign="end" wrap="wrap">
      {children}
    </HStack>
  );
}
