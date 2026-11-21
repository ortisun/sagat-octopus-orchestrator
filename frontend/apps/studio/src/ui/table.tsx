import { Table as AstryxTable, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow } from '@astryxdesign/core/Table';
import type { CSSProperties, ComponentProps, ReactNode } from 'react';

/**
 * A table of records, composed by hand: `Table` › `TableHead` / `TableBody` › `TableRow` ›
 * `ColumnHeader` / `RowHeader` / `TableCell`. It is a real `<table>`, so its rows and headers are announced as such.
 * `label` names it for assistive technology, as a caption did.
 */
export function Table({ label, children, density = 'compact', className }: { label: string; children: ReactNode; density?: 'compact' | 'balanced' | 'spacious'; className?: string }) {
  return (
    <div style={OWN_EDGES}>
      <AstryxTable aria-label={label} density={density} hasHover {...(className ? { className } : {})}>
        {children}
      </AstryxTable>
    </div>
  );
}

/**
 * An Astryx table placed first or last in a padded container (a card) pulls itself over that padding to
 * reach the container's edges. The Studio's tables sit inside a panel beside other content, often in a
 * scroll region of their own, where that pull hides the header row: an element around the table tells it there is no padding to
 * cross, so it keeps to its box.
 */
const OWN_EDGES = {
  '--container-padding-block-start': '0px',
  '--container-padding-block-end': '0px',
  '--container-padding-inline-start': '0px',
  '--container-padding-inline-end': '0px',
} as CSSProperties;

export const TableHead = TableHeader;

/** The cell that names its row (a `th` with the row scope): the name or the identifier of the record. */
export function RowHeader({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <TableHeaderCell scope="row" {...(className ? { className } : {})}>
      {children}
    </TableHeaderCell>
  );
}
export const ColumnHeader = TableHeaderCell;
export { TableBody, TableCell, TableRow };
export type TableCellProps = ComponentProps<typeof TableCell>;
