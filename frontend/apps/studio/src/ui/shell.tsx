import { AppShell } from '@astryxdesign/core/AppShell';
import { SideNav, SideNavHeading, SideNavItem, SideNavSection } from '@astryxdesign/core/SideNav';
import { TopNav } from '@astryxdesign/core/TopNav';
import type { ReactNode } from 'react';
import { RoutedAnchor } from './Link';

export interface NavEntry {
  to: string;
  label: string;
  current: boolean;
}

/**
 * The frame of every page (FR-007-009): the sidebar with the product's name and the navigation, the
 * top bar, and the page. `sidebar` is what sits between the brand and the entries of the chosen scope
 * (the namespace selector); `scopeLabel` names that scope for assistive technology.
 */
export function Frame({
  brand,
  tagline,
  mark,
  tenant,
  sidebar,
  scoped,
  scopeLabel,
  location,
  session,
  children,
}: {
  brand: string;
  tagline: string;
  mark: ReactNode;
  tenant: readonly NavEntry[];
  sidebar: ReactNode;
  scoped: readonly NavEntry[];
  scopeLabel: string;
  location: ReactNode;
  session: ReactNode;
  children: ReactNode;
}) {
  const item = (entry: NavEntry) => <SideNavItem key={entry.to} as={RoutedAnchor} href={entry.to} label={entry.label} isSelected={entry.current} />;
  return (
    <AppShell
      variant="elevated"
      sideNav={
        <SideNav aria-label="Main" header={<SideNavHeading heading={brand} subheading={tagline} icon={mark} />} topContent={sidebar}>
          <SideNavSection title="Tenant" isHeaderHidden>
            {tenant.map(item)}
          </SideNavSection>
          {scoped.length > 0 ? <SideNavSection title={scopeLabel}>{scoped.map(item)}</SideNavSection> : null}
        </SideNav>
      }
      topNav={<TopNav label="Current location and session" startContent={location} endContent={session} />}
    >
      {children}
    </AppShell>
  );
}
