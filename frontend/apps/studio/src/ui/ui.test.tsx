import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { describe, expect, it, vi } from 'vitest';
import { announcedAssertively, choose, chooseValue, chosen, expectAvailable, expectUnavailable, optionsOf, selector, valueOf } from '../../test/ui';
import { Actions, Badge, Banner, Button, Checkbox, ColumnHeader, Dialog, Empty, Field, Heading, Hint, Link, Loading, Select, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Tabs, TextArea, TextInput, UiProvider } from '.';

/** The contract of the interface module (Architecture decision 0101): what every screen relies on, whatever draws it. */
describe('Button', () => {
  it('is named by its text, or by a fuller aria-label, and reports a click', async () => {
    const user = userEvent.setup();
    const clicked = vi.fn();
    render(
      <>
        <Button onClick={clicked}>Save</Button>
        <Button aria-label="Drain worker identity w1">Drain identity</Button>
      </>,
    );
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(clicked).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Drain worker identity w1' })).toHaveTextContent('Drain identity');
  });

  it('when disabled is announced as unavailable and a click does nothing (Decision 2403)', async () => {
    const user = userEvent.setup();
    const clicked = vi.fn();
    render(
      <Button onClick={clicked} disabled>
        Publish
      </Button>,
    );
    const button = screen.getByRole('button', { name: 'Publish' });
    expectUnavailable(button);
    await user.click(button);
    expect(clicked).not.toHaveBeenCalled();
  });

  it('when disabled with an explanation stays reachable by keyboard, and Enter and Space do nothing', async () => {
    const user = userEvent.setup();
    const clicked = vi.fn();
    render(
      <Button onClick={clicked} disabled title="It has versions, which a workflow may pin.">
        Delete
      </Button>,
    );
    const button = screen.getByRole('button', { name: 'Delete' });
    expectUnavailable(button);
    button.focus();
    expect(button).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.keyboard(' ');
    await user.click(button);
    expect(clicked).not.toHaveBeenCalled();
  });

  it('submits its form when its type is submit, and not when it is disabled', async () => {
    const user = userEvent.setup();
    const submitted = vi.fn((event: { preventDefault: () => void }) => event.preventDefault());
    function Form() {
      const [ready, setReady] = useState(false);
      return (
        <form onSubmit={submitted}>
          <Button onClick={() => setReady(true)}>Make ready</Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            Send
          </Button>
        </form>
      );
    }
    render(<Form />);
    await user.click(screen.getByRole('button', { name: 'Send' }));
    expect(submitted).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Make ready' }));
    expectAvailable(screen.getByRole('button', { name: 'Send' }));
    await user.click(screen.getByRole('button', { name: 'Send' }));
    expect(submitted).toHaveBeenCalledTimes(1);
  });
});

describe('TextInput and TextArea', () => {
  it('tie the label, the hint and the announced error to the control and report what is typed', async () => {
    const user = userEvent.setup();
    function Form() {
      const [name, setName] = useState('');
      return <TextInput label="Name" hint="Lower-case letters." error={name === 'bad' ? 'That name is taken.' : undefined} value={name} onChange={setName} maxLength={5} />;
    }
    render(<Form />);
    const input = screen.getByLabelText('Name');
    expect(input).toHaveAccessibleDescription(expect.stringContaining('Lower-case letters.'));
    await user.type(input, 'bad');
    expect(input).toHaveValue('bad');
    expect(input).toBeInvalid();
    expect(input).toHaveAccessibleDescription(expect.stringContaining('That name is taken.'));
    // The error is announced when the person leaves the field, through the design system's assertive live region.
    await user.tab();
    await waitFor(() => expect(announcedAssertively()).toContain('That name is taken.'));
    await user.click(input);
    await user.type(input, 'dddd');
    expect(input).toHaveValue('baddd');
  });

  it('keep a password out of sight and a numeric field as the text that was typed', async () => {
    const user = userEvent.setup();
    const changed = vi.fn();
    render(
      <>
        <TextInput label="Password" type="password" value="s3cret" onChange={() => undefined} />
        <TextInput label="Port" numeric value="" onChange={changed} />
      </>,
    );
    expect(screen.getByLabelText('Password')).toHaveAttribute('type', 'password');
    await user.type(screen.getByLabelText('Port'), '5');
    expect(changed).toHaveBeenCalledWith('5');
    expect(screen.getByLabelText('Port')).toHaveAttribute('inputmode', 'numeric');
  });

  it('a text area drops what is typed or pasted beyond its limit', async () => {
    const user = userEvent.setup();
    function Form() {
      const [text, setText] = useState('abc');
      return <TextArea label="Notes" value={text} onChange={setText} maxLength={5} />;
    }
    render(<Form />);
    await user.click(screen.getByLabelText('Notes'));
    await user.paste('defgh');
    expect(screen.getByLabelText('Notes')).toHaveValue('abcde');
  });

  it('a text area reports its lines and does not accept typing when disabled', async () => {
    const user = userEvent.setup();
    const changed = vi.fn();
    render(
      <>
        <TextArea label="Reason" value="" onChange={changed} rows={2} />
        <TextArea label="Locked" value="kept" onChange={changed} disabled />
      </>,
    );
    await user.type(screen.getByLabelText('Reason'), 'a');
    expect(changed).toHaveBeenCalledWith('a');
    changed.mockClear();
    expectUnavailable(screen.getByLabelText('Locked'));
    await user.type(screen.getByLabelText('Locked'), 'x');
    expect(changed).not.toHaveBeenCalled();
  });
});

describe('Select', () => {
  it('shows the placeholder, offers its options and reports the choice', async () => {
    const user = userEvent.setup();
    function Form() {
      const [value, setValue] = useState<'a' | 'b' | ''>('');
      return (
        <>
          <Select
            label="Kind"
            placeholder="Choose a kind"
            value={value}
            onChange={setValue}
            options={[
              { value: 'a', label: 'Alpha' },
              { value: 'b', label: 'Beta' },
            ]}
          />
          <output>{value}</output>
        </>
      );
    }
    const { container } = render(<Form />);
    expect(chosen('Kind')).toContain('Choose a kind');
    expect(valueOf('Kind')).toBe('');
    expect(await optionsOf(user, 'Kind')).toEqual(['Alpha', 'Beta']);
    await user.keyboard('{Escape}');
    await choose(user, 'Kind', 'Beta');
    expect(container.querySelector('output')).toHaveTextContent('b');
    expect(chosen('Kind')).toContain('Beta');
    expect(valueOf('Kind')).toBe('b');
    await chooseValue(user, 'Kind', 'a');
    expect(valueOf('Kind')).toBe('a');
    expect(container.querySelector('output')).toHaveTextContent('a');
  });

  it('does not open when disabled', async () => {
    const user = userEvent.setup();
    render(<Select label="Kind" value="a" onChange={() => undefined} options={[{ value: 'a', label: 'Alpha' }]} disabled />);
    await user.click(selector('Kind'));
    expect(screen.queryByRole('option')).not.toBeInTheDocument();
  });
});

describe('Checkbox and Field', () => {
  it('a checkbox reports its new state', async () => {
    const user = userEvent.setup();
    const changed = vi.fn();
    render(<Checkbox label="Include archived" checked={false} onChange={changed} />);
    await user.click(screen.getByRole('checkbox', { name: 'Include archived' }));
    expect(changed).toHaveBeenCalledWith(true);
  });

  it('a checkbox ties what is missing to itself', () => {
    render(<Checkbox label="I know the steps run again" checked={false} onChange={() => undefined} error="Confirm that you know." />);
    const box = screen.getByRole('checkbox', { name: /I know the steps run again/ });
    expect(box).toBeInvalid();
    expect(box).toHaveAccessibleDescription(expect.stringContaining('Confirm that you know.'));
  });

  it('a field labels and describes a control that is not one of the module’s own', () => {
    render(
      <Field label="Source" hint="JavaScript." error="It does not parse.">
        <div role="textbox" tabIndex={0} />
      </Field>,
    );
    const control = screen.getByRole('textbox', { name: 'Source' });
    expect(control).toHaveAccessibleDescription(expect.stringContaining('JavaScript.'));
    expect(control).toHaveAccessibleDescription(expect.stringContaining('It does not parse.'));
    expect(control).toHaveAttribute('aria-invalid', 'true');
  });
});

describe('Dialog', () => {
  it('is a modal dialog named by its title, shows its content only while open, and asks to close on Escape', async () => {
    const user = userEvent.setup();
    function Owner() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <Button onClick={() => setOpen(true)}>Open</Button>
          <Dialog open={open} title="Publish version" onClose={() => setOpen(false)}>
            <p>Consequences</p>
            <Actions>
              <Button onClick={() => setOpen(false)}>Cancel</Button>
            </Actions>
          </Dialog>
        </>
      );
    }
    render(<Owner />);
    expect(screen.queryByText('Consequences')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open' }));
    const dialog = screen.getByRole('dialog', { name: 'Publish version' });
    expect(within(dialog).getByText('Consequences')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByText('Consequences')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Publish version' })).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText('Consequences')).not.toBeInTheDocument();
  });
});

describe('Dialog: where focus goes when it opens', () => {
  it('goes to the first control of the body that can be used, as the platform dialog does', async () => {
    render(
      <Dialog open title="Pause the schedule" onClose={() => undefined}>
        <p>Runs stop until it is resumed.</p>
        <TextInput label="Locked" value="" onChange={() => undefined} disabled />
        <TextInput label="Why" value="" onChange={() => undefined} />
      </Dialog>,
    );
    await waitFor(() => expect(screen.getByLabelText('Why')).toHaveFocus());
  });

  it('goes to the title when the body has nothing to use', async () => {
    render(
      <Dialog open title="Connection probe" onClose={() => undefined}>
        <p>Probing…</p>
      </Dialog>,
    );
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Connection probe' })).toHaveFocus());
  });
});

describe('Dialog: the close button', () => {
  it('has a close button of its own, for a body that offers no way out', async () => {
    const user = userEvent.setup();
    const closed = vi.fn();
    render(
      <Dialog open title="Connection probe" onClose={closed}>
        <p>Probing…</p>
      </Dialog>,
    );
    await user.click(within(screen.getByRole('dialog', { name: 'Connection probe' })).getByRole('button', { name: 'Close Connection probe' }));
    expect(closed).toHaveBeenCalledTimes(1);
  });
});

describe('Banner, Badge and the words of a state', () => {
  it('announces an error and a warning, and not information (Decision 2404)', () => {
    render(
      <>
        <Banner tone="error" title="Saving failed.">
          Try again.
        </Banner>
        <Banner tone="warn">Your session ended.</Banner>
        <Banner tone="info">Nothing to do.</Banner>
        <Banner tone="ok">Saved.</Banner>
      </>,
    );
    const alerts = screen.getAllByRole('alert');
    expect(alerts.map((alert) => alert.textContent)).toEqual([expect.stringContaining('Saving failed.'), expect.stringContaining('Your session ended.')]);
    expect(alerts[0]).toHaveTextContent('Try again.');
    expect(screen.getAllByRole('status').map((status) => status.textContent)).toEqual([expect.stringContaining('Nothing to do.'), expect.stringContaining('Saved.')]);
  });

  it('shows a state as a symbol and a word, with its tone as data and its help as a title', () => {
    render(
      <>
        <StatusBadge view={{ label: 'Failed', symbol: '✕', tone: 'bad', help: 'The step failed.' }} />
        <Badge tone="waiting">Drained</Badge>
      </>,
    );
    const failed = screen.getByTitle('The step failed.');
    expect(failed).toHaveAttribute('data-tone', 'bad');
    expect(failed).toHaveTextContent('✕');
    expect(failed).toHaveTextContent('Failed');
    expect(screen.getByText('Drained').closest('[data-tone]')).toHaveAttribute('data-tone', 'waiting');
  });
});

describe('Tabs', () => {
  it('follow the tabs pattern: one panel, arrows move the focus and Enter selects', async () => {
    const user = userEvent.setup();
    function Owner() {
      const [selected, setSelected] = useState('graph');
      return (
        <Tabs
          label="Views"
          selected={selected}
          onSelect={setSelected}
          tabs={[
            { id: 'graph', label: 'Graph', panel: <p>The graph</p> },
            { id: 'table', label: 'Table', panel: <p>The table</p> },
          ]}
        />
      );
    }
    render(<Owner />);
    expect(screen.getByRole('tablist', { name: 'Views' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Graph' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel', { name: 'Graph' })).toHaveTextContent('The graph');
    await user.click(screen.getByRole('tab', { name: 'Table' }));
    expect(screen.getByRole('tabpanel', { name: 'Table' })).toHaveTextContent('The table');
    // An arrow moves the focus along the strip; Enter selects the focused tab.
    await user.keyboard('{ArrowLeft}');
    expect(screen.getByRole('tab', { name: 'Graph' })).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('tab', { name: 'Graph' })).toHaveAttribute('aria-selected', 'true');
  });
});

describe('Table, text and links', () => {
  it('a table is a named table with column headers and cells', () => {
    render(
      <Table label="Registered sessions">
        <TableHead>
          <TableRow>
            <ColumnHeader>Name</ColumnHeader>
            <ColumnHeader>In flight</ColumnHeader>
          </TableRow>
        </TableHead>
        <TableBody>
          <TableRow>
            <TableCell>worker-1</TableCell>
            <TableCell>2 / 4</TableCell>
          </TableRow>
        </TableBody>
      </Table>,
    );
    const table = screen.getByRole('table', { name: 'Registered sessions' });
    expect(within(table).getAllByRole('columnheader').map((header) => header.textContent)).toEqual(['Name', 'In flight']);
    expect(within(table).getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['worker-1', '2 / 4']);
  });

  it('a heading keeps its level in the outline whatever it looks like', () => {
    render(
      <>
        <Heading level={1}>Runs</Heading>
        <Heading level={3} look={5}>
          Attempts
        </Heading>
        <Hint>Newest first.</Hint>
        <Empty title="No runs yet." />
        <Loading>Loading runs…</Loading>
      </>,
    );
    expect(screen.getByRole('heading', { level: 1, name: 'Runs' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 3, name: 'Attempts' })).toBeInTheDocument();
    expect(screen.getByText('Newest first.')).toBeInTheDocument();
    expect(screen.getByText('No runs yet.')).toBeInTheDocument();
    expect(screen.getByText('Loading runs…').closest('[role="status"]')).not.toBeNull();
  });

  it('a link navigates through the router without leaving the page', async () => {
    const user = userEvent.setup();
    render(
      <UiProvider>
        <MemoryRouter initialEntries={['/runs']}>
          <Routes>
            <Route path="/runs" element={<Link to="/runs/r1">Open run r1</Link>} />
            <Route path="/runs/:id" element={<p>The run</p>} />
          </Routes>
        </MemoryRouter>
      </UiProvider>,
    );
    const link = screen.getByRole('link', { name: 'Open run r1' });
    expect(link).toHaveAttribute('href', '/runs/r1');
    await user.click(link);
    expect(screen.getByText('The run')).toBeInTheDocument();
  });
});
