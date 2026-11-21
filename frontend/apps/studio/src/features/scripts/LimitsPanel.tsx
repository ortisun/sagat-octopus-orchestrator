import { ColumnHeader, Stack, Table, TableBody, TableCell, TableRow, Text } from '../../ui';
import type { ScriptPolicy } from './api';
import { formatDuration } from './text';

/** The limits a script runs under, as the platform stores them. The Studio shows them; it does not change them. */
export function LimitsPanel({ policy, retentionDays, language }: { policy: ScriptPolicy; retentionDays: number; language: string }) {
  return (
    <Stack gap={3}>
      <Text>These are the limits the platform holds the script to, in a test run and when it runs in a workflow. The platform refuses to ask for more than its own ceiling. The Studio does not change them.</Text>
      <Table label="Limits of the script">
        <TableBody>
          <TableRow>
            <ColumnHeader scope="row">Language</ColumnHeader>
            <TableCell>{language}</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Time (wall clock)</ColumnHeader>
            <TableCell>{formatDuration(policy.timeoutMs)}</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Memory</ColumnHeader>
            <TableCell>{policy.memoryMib} MiB</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">CPU</ColumnHeader>
            <TableCell>{policy.cpuMillis} millicores</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Processes</ColumnHeader>
            <TableCell>{policy.processes}</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Log kept</ColumnHeader>
            <TableCell>{policy.outputBytes.toLocaleString()} bytes</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Scratch space</ColumnHeader>
            <TableCell>{policy.scratchMib} MiB</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Network</ColumnHeader>
            <TableCell>{policy.egress === 'none' ? 'none: the script has no way out' : 'through octopus.fetch, under the namespace’s egress policy'}</TableCell>
          </TableRow>
          <TableRow>
            <ColumnHeader scope="row">Records kept for</ColumnHeader>
            <TableCell>{retentionDays} days</TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Stack>
  );
}
