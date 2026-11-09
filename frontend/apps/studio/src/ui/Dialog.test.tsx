import { act, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Dialog } from './Dialog';

function Harness({ onClose }: { onClose: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open it
      </button>
      <Dialog
        open={open}
        title="A dialog"
        onClose={() => {
          onClose();
          setOpen(false);
        }}
      >
        <button type="button">Inside</button>
      </Dialog>
    </>
  );
}

const dialog = () => document.querySelector('dialog') as HTMLDialogElement;

describe('The dialog', () => {
  it('gives focus back to the control that opened it when it closes', () => {
    render(<Harness onClose={() => undefined} />);
    const opener = screen.getByRole('button', { name: 'Open it' });
    opener.focus();
    fireEvent.click(opener);
    expect(dialog().open).toBe(true);
    // something inside holds focus, as it does once a person has moved in
    screen.getByRole('button', { name: 'Inside', hidden: true }).focus();
    fireEvent(dialog(), new Event('cancel', { cancelable: true }));
    expect(dialog().open).toBe(false);
    expect(opener).toHaveFocus();
  });

  it('is not closed again by the close event of an earlier closing: Escape and Enter on the opener in quick succession reopen it for good', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    const opener = screen.getByRole('button', { name: 'Open it' });
    opener.focus();
    fireEvent.click(opener);
    fireEvent(dialog(), new Event('cancel', { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog().open).toBe(false);
    // the person presses Enter on the opener before the browser has fired the `close` event of the closing that Escape made
    fireEvent.click(opener);
    expect(dialog().open).toBe(true);
    // and then the browser fires it: a dialog that is open now was not closed by it
    act(() => {
      dialog().dispatchEvent(new Event('close'));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog().open).toBe(true);
  });

  it('tells the owner when it closed itself, which the owner did not ask for', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open it' }));
    expect(dialog().open).toBe(true);
    // the browser closed it (a form with method="dialog"): the element is closed and the owner still thinks it is open
    act(() => {
      dialog().removeAttribute('open');
      dialog().dispatchEvent(new Event('close'));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
