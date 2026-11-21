import { Button as AstryxButton } from '@astryxdesign/core/Button';
import { Dialog as AstryxDialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { HStack } from '@astryxdesign/core/HStack';
import { VStack } from '@astryxdesign/core/VStack';
import type { ReactNode } from 'react';

/**
 * A modal dialog. Astryx draws it on the platform's own `<dialog>`: it traps focus, closes on Escape,
 * makes the page behind it inert and returns focus to what opened it. The owner decides when it is
 * open; `onClose` is the request to close it (Escape, or the close button in its header).
 */
export function Dialog({ open, title, onClose, children, width }: { open: boolean; title: string; onClose: () => void; children: ReactNode; width?: number | string }) {
  return (
    <AstryxDialog
      isOpen={open}
      purpose="form"
      {...(width !== undefined ? { width } : { width: 560 })}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogHeader
        title={title}
        // A way out that does not depend on the body: some bodies offer no button of their own. It is
        // named after the dialog so that it is not mistaken for a "Close" button of the body.
        endContent={
          <AstryxButton variant="ghost" size="sm" label={`Close ${title}`} onClick={onClose}>
            <span aria-hidden="true">✕</span>
          </AstryxButton>
        }
      />
      {open ? (
        <VStack gap={3} padding={4}>
          {children}
        </VStack>
      ) : null}
    </AstryxDialog>
  );
}

/** The row of a dialog's or a form's actions: the safe one first, the one that commits last. */
export function Actions({ children }: { children: ReactNode }) {
  return (
    <HStack gap={2} hAlign="end" wrap="wrap">
      {children}
    </HStack>
  );
}
