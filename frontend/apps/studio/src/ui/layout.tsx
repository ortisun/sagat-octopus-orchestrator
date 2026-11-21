import { Card } from '@astryxdesign/core/Card';
import { Collapsible } from '@astryxdesign/core/Collapsible';
import { Divider as AstryxDivider } from '@astryxdesign/core/Divider';
import { HStack } from '@astryxdesign/core/HStack';
import { MetadataList, MetadataListItem } from '@astryxdesign/core/MetadataList';
import { Spinner } from '@astryxdesign/core/Spinner';
import { VStack } from '@astryxdesign/core/VStack';
import type { ReactNode } from 'react';
import { Heading, Hint } from './text';

type Gap = 0 | 0.5 | 1 | 1.5 | 2 | 3 | 4 | 5 | 6 | 8;

/** Children one under the other, with the theme's spacing between them. */
export function Stack({ children, gap = 3, className, as, ...rest }: { children: ReactNode; gap?: Gap; className?: string; as?: 'div' | 'section' | 'form' | 'ul' | 'li'; id?: string; role?: string; 'aria-label'?: string; 'aria-labelledby'?: string }) {
  return (
    <VStack gap={gap} {...(as ? { as } : {})} {...(className ? { className } : {})} {...rest}>
      {children}
    </VStack>
  );
}

/** Children side by side; they wrap when the row is too narrow (reflow at 320px, WCAG 1.4.10). */
export function Row({ children, gap = 2, align = 'center', justify = 'start', wrap = true, className, ...rest }: { children: ReactNode; gap?: Gap; align?: 'start' | 'center' | 'end' | 'stretch'; justify?: 'start' | 'center' | 'end' | 'between'; wrap?: boolean; className?: string; role?: string; 'aria-label'?: string }) {
  return (
    <HStack gap={gap} vAlign={align} hAlign={justify} wrap={wrap ? 'wrap' : 'nowrap'} {...(className ? { className } : {})} {...rest}>
      {children}
    </HStack>
  );
}

/** The top of a page: its title (the page's `h1`), what it is for, and its actions. */
export function PageHeader({ title, children, actions }: { title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <HStack gap={3} vAlign="start" hAlign="between" wrap="wrap">
      <VStack gap={1}>
        <Heading level={1}>{title}</Heading>
        {children ? <Hint>{children}</Hint> : null}
      </VStack>
      {actions ? (
        <HStack gap={2} wrap="wrap">
          {actions}
        </HStack>
      ) : null}
    </HStack>
  );
}

/** A page of the Studio: the column every screen's content sits in. The shell's page area gives the padding. */
export function Page({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <VStack gap={4} {...(className ? { className } : {})}>
      {children}
    </VStack>
  );
}

/** A titled group of a page, drawn as a card. The title is a heading of the page's outline. */
export function Panel({ title, level = 2, hint, actions, children, labelledBy, className }: { title?: ReactNode; level?: 2 | 3 | 4; hint?: ReactNode; actions?: ReactNode; children: ReactNode; labelledBy?: string; className?: string }) {
  return (
    <Card {...(className ? { className } : {})}>
      <VStack gap={3} as="section" {...(labelledBy ? { 'aria-labelledby': labelledBy } : {})}>
        {title !== undefined || actions ? (
          <HStack gap={3} vAlign="start" hAlign="between" wrap="wrap">
            <VStack gap={1}>
              {title !== undefined ? (
                <Heading level={level} look={3} {...(labelledBy ? { id: labelledBy } : {})}>
                  {title}
                </Heading>
              ) : null}
              {hint ? <Hint>{hint}</Hint> : null}
            </VStack>
            {actions ? (
              <HStack gap={2} wrap="wrap">
                {actions}
              </HStack>
            ) : null}
          </HStack>
        ) : null}
        {children}
      </VStack>
    </Card>
  );
}

/** Named facts about one thing: a term and its value per row. */
export function Facts({ items, columns = 'single', label }: { items: readonly { term: string; value: ReactNode }[]; columns?: 'single' | 'multi'; label?: string }) {
  return (
    <MetadataList columns={columns} {...(label ? { 'aria-label': label } : {})}>
      {items.map((item) => (
        <MetadataListItem key={item.term} label={item.term}>
          {item.value}
        </MetadataListItem>
      ))}
    </MetadataList>
  );
}

/** Content behind a toggle. It starts closed unless `open`. */
export function Disclosure({ summary, children, open = false, onToggle }: { summary: ReactNode; children: ReactNode; open?: boolean; onToggle?: (open: boolean) => void }) {
  return (
    <Collapsible trigger={summary} defaultIsOpen={open} chevronPosition="start" {...(onToggle ? { onOpenChange: onToggle } : {})}>
      {children}
    </Collapsible>
  );
}

export function Divider() {
  return <AstryxDivider />;
}

/** Something is on its way. The words are announced; the spinner is decoration. */
export function Loading({ children }: { children: string }) {
  return (
    <HStack gap={2} vAlign="center" role="status">
      <Spinner size="sm" aria-hidden="true" />
      <span>{children}</span>
    </HStack>
  );
}
