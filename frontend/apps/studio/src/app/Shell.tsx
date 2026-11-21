import { useEffect } from 'react';
import { Outlet, useLocation } from 'react-router';
import { Badge, Button, Frame, Hint, Row, Select, Stack, Text } from '../ui';
import type { NavEntry } from '../ui';
import { useApp, useNamespace, useSession } from './context';
import { useQuery } from '@tanstack/react-query';

/** What belongs to the tenant, whatever namespace is chosen: the namespaces themselves. */
const TENANT = [{ to: '/namespaces', label: 'Namespaces' }] as const;

/** What belongs to one namespace: it is in the menu only while a namespace is chosen (UX review 2026-10-14, owner request). */
export const SCOPED = [
  { to: '/workflows', label: 'Workflows' },
  { to: '/scripts', label: 'Scripts' },
  { to: '/runs', label: 'Runs' },
  { to: '/operations', label: 'Operations' },
  { to: '/database-executions', label: 'Ad hoc database' },
  { to: '/connections', label: 'Connections' },
  { to: '/secrets', label: 'Secrets' },
  { to: '/integrations', label: 'Integrations' },
] as const;

const at = (pathname: string, to: string) => pathname === to || pathname.startsWith(`${to}/`);

function pageName(pathname: string): string {
  return [...SCOPED, ...TENANT].find((item) => at(pathname, item.to))?.label ?? 'Studio';
}

/**
 * The frame of every page: skip link, header with the namespace and the session, navigation, main
 * (drawn by the interface module's `Frame`, on the Astryx application shell).
 * The menu follows the hierarchy: the tenant's *Namespaces*, then the chosen namespace and, under it,
 * what lives in it. The Studio never chooses a namespace for the person, so until one is chosen the menu
 * offers only the namespaces.
 */
export function Shell() {
  const { session, ep, namespace } = useApp();
  const { info } = useSession();
  const current = useNamespace();
  const location = useLocation();
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });

  // A choice remembered from an earlier visit that the tenant does not list any more is forgotten, not replaced.
  const items = namespaces.data?.items;
  useEffect(() => {
    if (items !== undefined && current !== undefined && !items.some((n) => n.slug === current)) namespace.set(undefined);
  }, [items, current, namespace]);
  // While the list is still on its way a remembered choice is the person's own, so its menu is shown.
  const chosen = current !== undefined && (items === undefined || items.some((n) => n.slug === current)) ? current : undefined;
  const scoped = SCOPED.some((item) => at(location.pathname, item.to));

  // After a navigation focus moves to the page, so a keyboard or screen-reader user starts at its top.
  // The page's landmark is the application shell's own `main`, which holds the element below.
  useEffect(() => {
    const page = document.getElementById('main');
    (page?.closest<HTMLElement>('[role="main"]') ?? page)?.focus({ preventScroll: true });
  }, [location.pathname]);

  const entry = (item: { to: string; label: string }): NavEntry => ({ ...item, current: at(location.pathname, item.to) });
  const options = [
    ...(items === undefined && chosen !== undefined ? [{ value: chosen, label: chosen }] : []),
    ...(items ?? []).map((item) => ({ value: item.slug, label: item.slug })),
  ];

  return (
    <Frame
      brand="sagat-octopus"
      tagline="Durable sagas · many arms"
      mark={<span className="shell-mark" aria-hidden="true">S</span>}
      tenant={TENANT.map(entry)}
      scoped={chosen !== undefined ? SCOPED.map(entry) : []}
      scopeLabel={chosen !== undefined ? `In ${chosen}` : 'In a namespace'}
      sidebar={
        <Stack gap={2}>
          <Select
            label="Namespace"
            value={chosen ?? ''}
            onChange={(slug) => namespace.set(slug)}
            options={options}
            placeholder={items?.length === 0 ? 'No namespaces yet' : 'Choose a namespace'}
            disabled={items?.length === 0}
          />
          {chosen === undefined ? <Hint>Choose a namespace to open its workflows, runs, operations, scripts and integrations.</Hint> : null}
        </Stack>
      }
      location={
        <Row gap={1} aria-label="Current location" role="group">
          {scoped ? (
            <>
              <Text as="span" muted>{current ?? 'No namespace'}</Text>
              <span aria-hidden="true">/</span>
            </>
          ) : null}
          <strong>{pageName(location.pathname)}</strong>
        </Row>
      }
      session={
        <Row gap={2}>
          <Row gap={1} aria-label="Signed in as" role="group">
            <span>{info?.subject}</span>{' '}
            <Badge>{info?.role}</Badge>
          </Row>
          <Button onClick={() => session.signOut('signed-out')}>Sign out</Button>
        </Row>
      }
    >
      <div id="main" tabIndex={-1} className="shell-main">
        <Outlet />
      </div>
    </Frame>
  );
}
