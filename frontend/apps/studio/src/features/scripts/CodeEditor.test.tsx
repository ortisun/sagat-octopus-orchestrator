import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CodeEditor } from './CodeEditor';
import type { MonacoEditorProps } from './MonacoEditor';
import { stubStorage } from './testing';

const loader = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock('./editorModule', () => ({ loadEditor: () => loader.load() as unknown }));

/** Stands in for Monaco, which jsdom cannot run: it shows what the wrapper handed it. */
function StandIn(props: MonacoEditorProps) {
  return (
    <div data-testid="monaco" data-model={props.model} data-language={props.language} data-readonly={String(props.readOnly)} data-tab-moves-focus={String(props.tabMovesFocus)} aria-label={props.ariaLabel} role="group">
      <pre>{props.value}</pre>
      <button type="button" onClick={() => props.onChange?.('typed in monaco')}>
        type in monaco
      </button>
    </div>
  );
}

function Harness({ initial = 'start', readOnly = false }: { initial?: string; readOnly?: boolean }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <CodeEditor label="Source" model="source" language="typescript" value={value} onChange={setValue} readOnly={readOnly} />
      <p data-testid="value">{value}</p>
      <button type="button">after the editor</button>
    </>
  );
}

let storage: Storage;
beforeEach(() => {
  loader.load.mockReset();
  storage = stubStorage();
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('CodeEditor without Monaco', () => {
  it('falls back to a plain text box with the same text when the editor cannot be loaded, and says so', async () => {
    loader.load.mockRejectedValue(new Error('blocked worker'));
    render(<Harness initial="export default 1;" />);
    const box = await screen.findByRole('textbox', { name: 'Source' });
    expect(box).toHaveValue('export default 1;');
    expect(screen.getByRole('alert')).toHaveTextContent('The code editor could not be loaded.');
    expect(screen.queryByTestId('monaco')).not.toBeInTheDocument();
    const user = userEvent.setup();
    await user.clear(box);
    await user.type(box, 'changed');
    expect(screen.getByTestId('value')).toHaveTextContent('changed');
  });

  it('is a read-only box when the text may not be changed', async () => {
    loader.load.mockRejectedValue(new Error('no'));
    render(<Harness readOnly />);
    expect(await screen.findByRole('textbox', { name: 'Source' })).toHaveAttribute('readonly');
  });

  it('says it is loading while it loads, and a plain box can be chosen at once', async () => {
    loader.load.mockReturnValue(new Promise(() => undefined));
    render(<Harness />);
    expect(screen.getByRole('status')).toHaveTextContent('Loading the code editor…');
    await userEvent.setup().click(screen.getByRole('button', { name: 'Use a plain text box' }));
    expect(screen.getByRole('textbox', { name: 'Source' })).toHaveValue('start');
  });

  it('does not touch the editor chunk when the person prefers the plain box, and remembers the choice', async () => {
    loader.load.mockResolvedValue({ MonacoEditor: StandIn });
    const user = userEvent.setup();
    const first = render(<Harness />);
    await user.click(await screen.findByRole('button', { name: 'Use a plain text box' }));
    expect(screen.getByRole('textbox', { name: 'Source' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Use the code editor' })).toHaveAttribute('aria-pressed', 'true');
    first.unmount();
    loader.load.mockClear();
    render(<Harness />);
    expect(screen.getByRole('textbox', { name: 'Source' })).toBeInTheDocument();
    expect(loader.load).not.toHaveBeenCalled();
    // And back.
    await user.click(screen.getByRole('button', { name: 'Use the code editor' }));
    expect(await screen.findByTestId('monaco')).toBeInTheDocument();
    expect(storage.getItem('studio.scripts.plainEditor')).toBeNull();
  });

  it('works when the browser will not keep the preference', async () => {
    loader.load.mockRejectedValue(new Error('no'));
    vi.spyOn(storage, 'setItem').mockImplementation(() => {
      throw new Error('storage is blocked');
    });
    render(<Harness />);
    await userEvent.setup().click(await screen.findByRole('button', { name: 'Use a plain text box' }));
    expect(screen.getByRole('textbox', { name: 'Source' })).toBeInTheDocument();
  });
});

describe('CodeEditor with Monaco', () => {
  it('hands the text, the language, the model and the name to the editor, and takes its changes', async () => {
    loader.load.mockResolvedValue({ MonacoEditor: StandIn });
    render(<Harness initial="let x = 1;" />);
    const monaco = await screen.findByTestId('monaco');
    expect(monaco).toHaveAttribute('data-model', 'source');
    expect(monaco).toHaveAttribute('data-language', 'typescript');
    expect(monaco).toHaveAttribute('data-readonly', 'false');
    expect(monaco).toHaveAccessibleName('Source');
    expect(monaco).toHaveTextContent('let x = 1;');
    await userEvent.setup().click(screen.getByRole('button', { name: 'type in monaco' }));
    expect(screen.getByTestId('value')).toHaveTextContent('typed in monaco');
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  });

  it('passes read-only on', async () => {
    loader.load.mockResolvedValue({ MonacoEditor: StandIn });
    render(<Harness readOnly />);
    expect(await screen.findByTestId('monaco')).toHaveAttribute('data-readonly', 'true');
  });

  it('tells how to leave the editor with the keyboard, and offers it as a switch and a button', async () => {
    loader.load.mockResolvedValue({ MonacoEditor: StandIn });
    const user = userEvent.setup();
    render(<Harness />);
    await screen.findByTestId('monaco');
    expect(screen.getByText(/Tab key inserts a tab/)).toBeInTheDocument();
    expect(screen.getAllByText('M', { selector: 'kbd' })).toHaveLength(2);
    const toggle = screen.getByRole('checkbox', { name: 'Tab moves focus out of the editor' });
    expect(screen.getByTestId('monaco')).toHaveAttribute('data-tab-moves-focus', 'false');
    await user.click(toggle);
    expect(screen.getByTestId('monaco')).toHaveAttribute('data-tab-moves-focus', 'true');

    // "Skip editor" puts focus after the editor, so the next Tab goes on to the page.
    await user.click(screen.getByRole('button', { name: 'Skip editor' }));
    await waitFor(() => expect(screen.getByText('End of the code editor')).toHaveFocus());
    await user.tab();
    expect(screen.getByRole('button', { name: 'after the editor' })).toHaveFocus();
  });

  it('gives the plain box no keyboard trap to explain', async () => {
    loader.load.mockRejectedValue(new Error('no'));
    render(<Harness />);
    await screen.findByRole('textbox');
    expect(screen.queryByText(/Tab key inserts a tab/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Skip editor' })).not.toBeInTheDocument();
  });
});
