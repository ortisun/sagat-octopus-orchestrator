import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { scriptsConfig } from './config';
import type { scriptsServer} from './testing';
import { DEVELOPER, RUN_ID, SCRIPT_ID, aRun, aRunSummary, aScript, json, problem, stubStorage } from './testing';
import type { ScriptTestRun } from './api';
import { alerts, openEditor, patchesOf } from './testingEditor';
import { eventually, jump, manualTime, restoreTime } from './testingTime';

vi.mock('./editorModule', () => ({ loadEditor: () => Promise.reject(new Error('no Monaco in jsdom')) }));

const ESC = String.fromCharCode(27);

beforeEach(() => {
  stubStorage();
  scriptsConfig.autosaveMs = 30;
  // Time moves by itself, so the page's own waits work, and the tests can also jump ahead.
  manualTime();
});
afterEach(() => {
  scriptsConfig.autosaveMs = 1500;
  restoreTime();
  vi.unstubAllGlobals();
});


/** The platform answers the run with `answers` in order, the last one for ever. */
function scriptRun(server: ReturnType<typeof scriptsServer>, answers: ScriptTestRun[], created: ScriptTestRun = aRun({ status: 'QUEUED' })) {
  let n = 0;
  server.override(`POST ${server.paths.one}/test-runs`, () => json(202, created));
  server.override(`GET ${server.paths.one}/test-runs/${RUN_ID}`, () => answers[Math.min(n++, answers.length - 1)]);
  return { reads: () => server.callsTo('GET', `${server.paths.one}/test-runs/${RUN_ID}`).length };
}

const runButton = () => eventually(() => screen.getByRole('button', { name: 'Run in the sandbox' }));
const sourceBox = () => eventually(() => screen.getByRole('textbox', { name: 'Source of double-it (TypeScript)' }));
const resultRegion = () => screen.getByTestId('test-result');

describe('Try it in the sandbox: a run from start to end', () => {
  it('asks for the run with the input in the box, then watches it once a second until the platform says it is final, and shows only what it said', async () => {
    const { server } = await openEditor();
    const polls = scriptRun(server, [aRun({ status: 'QUEUED' }), aRun({ status: 'RUNNING' }), aRun({ status: 'SUCCEEDED', result: { double: 42 }, log: `${ESC}[32mn = 21${ESC}[0m\n`, wallMs: 1200, startMs: 85, memoryPeakBytes: 25 * 1024 * 1024 })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());

    // The input in the box is what is sent.
    await eventually(() => expect(server.callsTo('POST', `${server.paths.one}/test-runs`)).toHaveLength(1));
    expect(JSON.parse(server.callsTo('POST', `${server.paths.one}/test-runs`)[0]?.init.body as string)).toEqual({ input: { n: 21 } });

    // Nothing is claimed before the platform has said it: queued means nothing has run.
    expect(await eventually(() => within(resultRegion()).getByText('Queued'))).toBeInTheDocument();
    expect(resultRegion()).toHaveTextContent('Nothing has run yet.');
    expect(screen.queryByRole('region', { name: 'Value the script returned' })).not.toBeInTheDocument();
    expect(polls.reads()).toBe(0);

    // Looked at once a second: still queued, then running, then done.
    await jump(1000);
    expect(polls.reads()).toBe(1);
    expect(within(resultRegion()).getByText('Queued')).toBeInTheDocument();

    await jump(1000);
    expect(polls.reads()).toBe(2);
    expect(within(resultRegion()).getByText('Running')).toBeInTheDocument();
    expect(resultRegion()).toHaveTextContent('There is no result yet.');
    expect(screen.queryByRole('region', { name: 'Log of the run' })).not.toBeInTheDocument();

    await jump(1000);
    expect(polls.reads()).toBe(3);
    expect(within(resultRegion()).getByText('Succeeded')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Value the script returned' })).toHaveTextContent('"double": 42');
    // The terminal escapes are gone; the words are kept.
    const log = screen.getByRole('region', { name: 'Log of the run' });
    expect(log).toHaveTextContent('n = 21');
    expect(log.textContent).not.toContain(ESC);
    expect(log.textContent).not.toContain('[32m');
    expect(resultRegion()).toHaveTextContent('1.2 s of 30 s allowed');
    expect(resultRegion()).toHaveTextContent('85 ms from asking for a container');
    expect(resultRegion()).toHaveTextContent('25 MiB of 256 MiB allowed');
    expect(resultRegion()).toHaveTextContent('does not report CPU time');
    expect(resultRegion()).toHaveTextContent(RUN_ID);
    expect(resultRegion()).toHaveTextContent('The script was not refused anything.');

    // Final: it is not asked again.
    await jump(30_000);
    expect(polls.reads()).toBe(3);
    expect(await eventually(() => screen.getByText('Test run succeeded.'))).toBeInTheDocument();
  });

  it('saves the draft first, because the sandbox runs the platform\'s copy', async () => {
    scriptsConfig.autosaveMs = 60_000;
    const { server } = await openEditor();
    scriptRun(server, [aRun({ status: 'SUCCEEDED', result: 1 })]);
    const user = userEvent.setup({ delay: null });
    await user.type(await sourceBox(), '// changed');
    expect(patchesOf(server)).toHaveLength(0);
    await user.click(await runButton());
    await eventually(() => expect(server.callsTo('POST', `${server.paths.one}/test-runs`)).toHaveLength(1));
    expect(patchesOf(server)).toHaveLength(1);
    const order = server.calls.filter((c) => c.method !== 'GET').map((c) => c.method + ' ' + c.url.split('/').slice(-1)[0]);
    expect(order).toEqual(['PATCH ' + SCRIPT_ID, 'POST test-runs']);
  });

  it('shows a failure with its code and what it means, what was refused, and nothing it was not told', async () => {
    const { server } = await openEditor();
    scriptRun(server, [aRun({ status: 'FAILED', failure: 'SCRIPT_DENIED', log: 'tried to reach the network\n', denied: ['fetch http://169.254.169.254', 'secret db-password'] })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(1000);
    expect(await eventually(() => within(resultRegion()).getByText('Failed'))).toBeInTheDocument();
    expect(alerts()).toContain('Failed: SCRIPT_DENIED.');
    expect(alerts()).toContain('asked for something its policy does not allow');
    expect(screen.queryByRole('region', { name: 'Value the script returned' })).not.toBeInTheDocument();
    expect(resultRegion()).toHaveTextContent('fetch http://169.254.169.254');
    expect(resultRegion()).toHaveTextContent('secret db-password');
    expect(screen.getByRole('region', { name: 'Log of the run' })).toHaveTextContent('tried to reach the network');
  });

  it('keeps the page whole when a log is a flood: it shows the start, says it left the rest out, and can show more', async () => {
    const { server } = await openEditor();
    const flood = Array.from({ length: 5000 }, (_, i) => `line ${i} ${ESC}[31m${'x'.repeat(30)}${ESC}[0m`).join('\n');
    scriptRun(server, [aRun({ status: 'SUCCEEDED', result: 'ok', log: flood, logTruncated: true })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(1000);
    const log = await eventually(() => screen.getByRole('region', { name: 'Log of the run' }));
    expect(log.textContent?.split('\n').length).toBeLessThanOrEqual(400);
    expect(log).toHaveTextContent('line 0 ');
    expect(log).not.toHaveTextContent('line 4999');
    expect(resultRegion()).toHaveTextContent(/Showing the start of .* characters \(5,000 lines\)/);
    expect(resultRegion()).toHaveTextContent('The platform cut this at the output limit of the script');
    await user.click(within(resultRegion()).getByRole('button', { name: 'Show more' }));
    expect(screen.getByRole('region', { name: 'Log of the run' })).toHaveTextContent('line 4999');
  });

  it('shows a result that holds markup as text', async () => {
    const { server } = await openEditor();
    scriptRun(server, [aRun({ status: 'SUCCEEDED', result: { html: '<script>alert(1)</script>' }, log: '<b>bold?</b>' })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(1000);
    expect(await eventually(() => screen.getByRole('region', { name: 'Value the script returned' }))).toHaveTextContent('<script>alert(1)</script>');
    expect(screen.getByRole('region', { name: 'Log of the run' })).toHaveTextContent('<b>bold?</b>');
    expect(document.querySelector('b')).toBeNull();
    expect(document.querySelector('script')).toBeNull();
  });
});

describe('Try it in the sandbox: the input', () => {
  it('does not run with an input that is not JSON, and says why', async () => {
    const { server } = await openEditor();
    const user = userEvent.setup({ delay: null });
    const input = await eventually(() => screen.getByRole('textbox', { name: 'Test input (JSON)' }));
    await user.clear(input);
    await user.click(input);
    await user.paste('{n: ');
    expect(await eventually(() => screen.getByText(/^Not valid JSON/))).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Run in the sandbox' })).toBeDisabled();
    expect(server.calls.filter((c) => c.url.includes('/test-runs') && c.method === 'POST')).toHaveLength(0);
  });

  it('sends nothing as the input when the box is empty, which the platform reads as null', async () => {
    const { server } = await openEditor();
    scriptRun(server, [aRun({ status: 'SUCCEEDED', result: null })]);
    const user = userEvent.setup({ delay: null });
    await user.clear(await eventually(() => screen.getByRole('textbox', { name: 'Test input (JSON)' })));
    await user.click(await runButton());
    await eventually(() => expect(server.callsTo('POST', `${server.paths.one}/test-runs`)).toHaveLength(1));
    await jump(1000);
    expect(JSON.parse(server.callsTo('POST', `${server.paths.one}/test-runs`)[0]?.init.body as string)).toEqual({});
    // A result of null is a result.
    expect(await eventually(() => screen.getByRole('region', { name: 'Value the script returned' }))).toHaveTextContent('null');
  });

  it('says whether the input is checked against the input schema', async () => {
    await openEditor(DEVELOPER, { script: aScript({ inputSchema: { type: 'object' } as never }) });
    expect(await eventually(() => screen.getByText(/Checked against the input schema of the draft as you type/))).toBeInTheDocument();
  });
});

describe('Try it in the sandbox: when it does not end', () => {
  it('stops looking when told to, and the run goes on in the platform', async () => {
    const { server } = await openEditor();
    const polls = scriptRun(server, [aRun({ status: 'RUNNING' })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(3000);
    expect(polls.reads()).toBe(3);
    await user.click(await eventually(() => screen.getByRole('button', { name: 'Stop watching' })));
    expect(screen.getByText('Not watching any more.')).toBeInTheDocument();
    const before = polls.reads();
    await jump(60_000);
    expect(polls.reads()).toBe(before);
    expect(screen.getByText(/Stopped checking the test run\. It goes on in the platform\./)).toBeInTheDocument();
  });

  it('gives up when the time for the run is up and does not claim an end it was not told', async () => {
    const { server } = await openEditor(DEVELOPER, { script: aScript({ policy: { memoryMib: 256, cpuMillis: 1000, timeoutMs: 1000, processes: 64, outputBytes: 65_536, scratchMib: 32, egress: 'none' } }) });
    const polls = scriptRun(server, [aRun({ status: 'QUEUED' })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    // The wait for a run is the script's time limit plus five minutes.
    await jump(6 * 60_000 + 10_000);
    expect(await eventually(() => screen.getByText('Still not final.'))).toBeInTheDocument();
    expect(screen.getByText(/a sandbox runner may be busy or not running/)).toBeInTheDocument();
    expect(within(resultRegion()).getByText('Queued')).toBeInTheDocument();
    const reads = polls.reads();
    expect(reads).toBeGreaterThan(10);
    await jump(10 * 60_000);
    expect(polls.reads()).toBe(reads);
    // "Check again" looks once more, and watches again. The run stays on screen while the platform is asked.
    const key = `GET ${server.paths.one}/test-runs/${RUN_ID}`;
    const release = server.hold(key);
    server.override(key, () => aRun({ status: 'FAILED', failure: 'SCRIPT_SANDBOX_UNAVAILABLE' }));
    await user.click(screen.getByRole('button', { name: 'Check again' }));
    const region = resultRegion();
    expect(within(region).getByText('Queued')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Result' })).toHaveFocus();
    expect(screen.queryByRole('button', { name: 'Check again' })).not.toBeInTheDocument();
    expect(within(region).getByRole('button', { name: 'Stop watching' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Run in the sandbox' })).toBeDisabled();
    release();
    expect(await eventually(() => within(resultRegion()).getByText('Failed'))).toBeInTheDocument();
    expect(alerts()).toContain('SCRIPT_SANDBOX_UNAVAILABLE');
  });

  it('stops looking when the page is left', async () => {
    const { server, view } = await openEditor();
    const polls = scriptRun(server, [aRun({ status: 'RUNNING' })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(2000);
    const before = polls.reads();
    expect(before).toBeGreaterThan(0);
    view.unmount();
    await jump(60_000);
    expect(polls.reads()).toBe(before);
  });

  it('keeps watching while another tab is open', async () => {
    const { server } = await openEditor();
    const polls = scriptRun(server, [aRun({ status: 'RUNNING' })]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await user.click(screen.getByRole('tab', { name: 'Limits' }));
    await jump(3000);
    expect(polls.reads()).toBe(3);
    await user.click(screen.getByRole('tab', { name: 'Try it in the sandbox' }));
    expect(within(resultRegion()).getByText('Running')).toBeInTheDocument();
  });

  it('says so when it cannot start a run, with the reason and the trace id', async () => {
    const { server } = await openEditor();
    server.override(`POST ${server.paths.one}/test-runs`, () => problem(429, 'QUOTA_EXCEEDED', 'a limit was reached; release something and retry', { limit: 'script_jobs' }));
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    expect(await eventually(() => screen.getByText('The test run failed.'))).toBeInTheDocument();
    expect(alerts()).toContain('a limit was reached');
    expect(alerts()).toContain('a'.repeat(32));
    expect(screen.getByRole('button', { name: 'Run in the sandbox' })).toBeEnabled();
  });

  it('puts up with one failed look and goes on', async () => {
    const { server } = await openEditor();
    let reads = 0;
    server.override(`POST ${server.paths.one}/test-runs`, () => json(202, aRun({ status: 'QUEUED' })));
    server.override(`GET ${server.paths.one}/test-runs/${RUN_ID}`, () => {
      reads += 1;
      return reads === 1 ? problem(503, 'UNAVAILABLE', 'the platform is not available right now') : aRun({ status: 'SUCCEEDED', result: 7 });
    });
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(1000);
    await jump(1000);
    expect(await eventually(() => within(resultRegion()).getByText('Succeeded'))).toBeInTheDocument();
  });
});

describe('Try it in the sandbox: earlier runs', () => {
  const earlier = aRunSummary({ id: '77777777-7777-4777-8777-777777777777', status: 'FAILED', failure: 'SCRIPT_TIMEOUT', wallMs: 30_000, createdBy: 'bob' });

  it('lists them, and opens one with everything it said', async () => {
    const { server } = await openEditor(DEVELOPER, { runs: [earlier, aRunSummary()] });
    server.override(`GET ${server.paths.one}/test-runs/${earlier.id}`, () => aRun({ ...earlier, status: 'FAILED', failure: 'SCRIPT_TIMEOUT', log: 'spinning\n', input: { n: 5 } }));
    const user = userEvent.setup({ delay: null });
    const table = await eventually(() => screen.getByRole('table', { name: 'Test runs of this script, newest first' }));
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('SCRIPT_TIMEOUT');
    expect(rows[0]).toHaveTextContent('30 s');
    expect(rows[0]).toHaveTextContent('bob');
    await user.click(within(rows[0]!).getByRole('button', { name: /Open the test run of/ }));
    expect(await eventually(() => screen.getByText('Failed: SCRIPT_TIMEOUT.'))).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Log of the run' })).toHaveTextContent('spinning');
    // Its input can be tried again.
    await user.click(screen.getByRole('button', { name: 'Use this run\'s input' }));
    expect(screen.getByRole('textbox', { name: 'Test input (JSON)' })).toHaveValue('{\n  "n": 5\n}\n');
  });

  it('shows that it is reading a run of the history instead of leaving the last one on screen as if it were that one', async () => {
    const { server } = await openEditor(DEVELOPER, { runs: [earlier, aRunSummary()] });
    const key = `GET ${server.paths.one}/test-runs/${earlier.id}`;
    const release = server.hold(key);
    server.override(key, () => aRun({ ...earlier, status: 'FAILED', failure: 'SCRIPT_TIMEOUT', log: 'spinning\n' }));
    scriptRun(server, [aRun({ status: 'SUCCEEDED', result: 1 })]);
    const user = userEvent.setup({ delay: null });
    // A run of this session is on screen first.
    await user.click(await runButton());
    await jump(1000);
    expect(within(resultRegion()).getByText('Succeeded')).toBeInTheDocument();
    const table = await eventually(() => screen.getByRole('table', { name: 'Test runs of this script, newest first' }));
    await user.click(within(within(table).getAllByRole('row')[1]!).getByRole('button', { name: /Open the test run of/ }));
    expect(screen.getByText('Loading the test run…')).toBeInTheDocument();
    expect(screen.queryByTestId('test-result')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Run in the sandbox' })).toBeDisabled();
    release();
    expect(await eventually(() => screen.getByText('Failed: SCRIPT_TIMEOUT.'))).toBeInTheDocument();
    expect(screen.queryByText('Loading the test run…')).not.toBeInTheDocument();
  });

  it('says when there are none', async () => {
    await openEditor();
    expect(await eventually(() => screen.getByText('No test runs yet.'))).toBeInTheDocument();
  });

  it('puts the inferred schemas in the schema editors, where they can be changed', async () => {
    const { server } = await openEditor();
    const inferred = aRun({ status: 'SUCCEEDED', result: { double: 42 }, inferredInputSchema: { type: 'object', properties: { n: { type: 'integer' } } } as never, inferredOutputSchema: { type: 'object', properties: { double: { type: 'integer' } } } as never });
    scriptRun(server, [inferred]);
    const user = userEvent.setup({ delay: null });
    await user.click(await runButton());
    await jump(1000);
    await user.click(await eventually(() => screen.getByRole('button', { name: 'Use as output schema' })));
    // The draft is saved a moment (the autosave pause) after the change.
    await jump(scriptsConfig.autosaveMs);
    await eventually(() => expect(patchesOf(server)).toHaveLength(1));
    expect(patchesOf(server)[0]).toEqual({ revision: 1, outputSchema: { type: 'object', properties: { double: { type: 'integer' } } } });
    await user.click(screen.getByRole('tab', { name: 'Input and output schemas' }));
    const box = await eventually(() => screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Output schema (JSON)' }));
    expect(box.value).toContain('"double"');
  });
});
