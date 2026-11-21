import { Table as AstryxTable } from '@astryxdesign/core/Table';
import type { ReactNode } from 'react';

/**
 * A table of which the page may hold only the rows in view (the screen windows them itself and passes
 * the rows it draws). `rowCount` is the number of rows there are in all, the header row included, so
 * assistive technology can say "row 40 of 5001"; each drawn row then carries its `aria-rowindex`.
 * Without `rowCount` it is the ordinary table.
 */
export function WindowedTable({ label, rowCount, children, className }: { label: string; rowCount?: number | undefined; children: ReactNode; className?: string }) {
  return (
    <AstryxTable aria-label={label} density="compact" hasHover {...(rowCount !== undefined ? { 'aria-rowcount': rowCount } : {})} {...(className ? { className } : {})}>
      {children}
    </AstryxTable>
  );
}
