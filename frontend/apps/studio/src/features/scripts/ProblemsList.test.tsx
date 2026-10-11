import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import type { RefObject } from 'react';
import { describe, expect, it, vi } from 'vitest';
import type { EditorControls } from './CodeEditor';
import type { Problem } from './monacoSetup';
import { ProblemsList } from './ProblemsList';

const problems: Problem[] = [
  { severity: 'error', line: 3, column: 5, message: "Cannot find name 'foo'." },
  { severity: 'warning', line: 9, column: 1, message: 'Unused.' },
];

function controls(): RefObject<EditorControls | null> {
  const ref = createRef<EditorControls | null>();
  (ref as { current: EditorControls | null }).current = { focus: vi.fn(), reveal: vi.fn() };
  return ref;
}

describe('ProblemsList', () => {
  it('lists what the editor underlines, with its place and its severity as words, since a squiggle is not for a keyboard', () => {
    render(<ProblemsList problems={problems} controlsRef={controls()} label="source" />);
    expect(screen.getByRole('heading', { name: /Problems/ })).toHaveTextContent('1 error, 1 other');
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent('Error');
    expect(items[0]).toHaveTextContent("Cannot find name 'foo'.");
    expect(items[1]).toHaveTextContent('Warning');
  });

  it('takes the cursor to a problem', async () => {
    const ref = controls();
    render(<ProblemsList problems={problems} controlsRef={ref} label="source" />);
    await userEvent.setup().click(screen.getByRole('button', { name: 'Line 3, column 5' }));
    expect(ref.current?.reveal).toHaveBeenCalledWith(3, 5);
  });

  it('says none were reported, not that there are none', () => {
    render(<ProblemsList problems={[]} controlsRef={controls()} label="source" />);
    expect(screen.getByRole('heading', { name: /Problems/ })).toHaveTextContent('none reported');
    expect(screen.queryByRole('list')).not.toBeInTheDocument();
  });

  it('shows a message as text', () => {
    render(<ProblemsList problems={[{ severity: 'info', line: 1, column: 1, message: '<img src=x onerror=alert(1)>' }]} controlsRef={controls()} label="source" />);
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
  });
});
