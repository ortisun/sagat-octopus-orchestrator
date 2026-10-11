import type { ScriptPolicy } from './api';
import { formatDuration } from './text';

/** The limits a script runs under, as the platform stores them. The Studio shows them; it does not change them. */
export function LimitsPanel({ policy, retentionDays, language }: { policy: ScriptPolicy; retentionDays: number; language: string }) {
  return (
    <div className="limits-panel">
      <p>These are the limits the platform holds the script to, in a test run and when it runs in a workflow. The platform refuses to ask for more than its own ceiling. The Studio does not change them.</p>
      <table className="data">
        <caption className="visually-hidden">Limits of the script</caption>
        <tbody>
          <tr>
            <th scope="row">Language</th>
            <td>{language}</td>
          </tr>
          <tr>
            <th scope="row">Time (wall clock)</th>
            <td>{formatDuration(policy.timeoutMs)}</td>
          </tr>
          <tr>
            <th scope="row">Memory</th>
            <td>{policy.memoryMib} MiB</td>
          </tr>
          <tr>
            <th scope="row">CPU</th>
            <td>{policy.cpuMillis} millicores</td>
          </tr>
          <tr>
            <th scope="row">Processes</th>
            <td>{policy.processes}</td>
          </tr>
          <tr>
            <th scope="row">Log kept</th>
            <td>{policy.outputBytes.toLocaleString()} bytes</td>
          </tr>
          <tr>
            <th scope="row">Scratch space</th>
            <td>{policy.scratchMib} MiB</td>
          </tr>
          <tr>
            <th scope="row">Network</th>
            <td>{policy.egress === 'none' ? 'none: the script has no way out' : 'through octopus.fetch, under the namespace’s egress policy'}</td>
          </tr>
          <tr>
            <th scope="row">Records kept for</th>
            <td>{retentionDays} days</td>
          </tr>
        </tbody>
      </table>
    </div>
  );
}
