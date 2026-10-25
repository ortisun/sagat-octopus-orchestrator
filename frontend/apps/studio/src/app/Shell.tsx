import { useEffect } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router';
import { Button } from '../ui/Button';
import { useApp, useNamespace, useSession } from './context';
import { useQuery } from '@tanstack/react-query';

/** What belongs to the tenant, whatever namespace is chosen: the namespaces themselves. */
const TENANT = [{ to: '/namespaces', label: 'Namespaces' }] as const;

/** What belongs to one namespace: it is in the menu only while a namespace is chosen (UX review 2026-10-14, owner request). */
export const SCOPED = [
  { to: '/workflows', label: 'Workflows' },
  { to: '/runs', label: 'Runs' },
  { to: '/operations', label: 'Operations' },
  { to: '/scripts', label: 'Scripts' },
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
 * The frame of every page: skip link, header with the namespace and the session, navigation, main.
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
  useEffect(() => {
    document.getElementById('main')?.focus({ preventScroll: true });
  }, [location.pathname]);

  return (
    <div className="shell">
      <a className="skip-link" href="#main">
        Skip to the content
      </a>
      <aside className="shell-sidebar" aria-label="Studio navigation">
        <div className="shell-brand">
          <span className="shell-mark" aria-hidden="true">S</span>
          <span>
            <strong>sagat-octopus</strong>
            <small>Durable sagas · many arms</small>
          </span>
        </div>
        <nav aria-label="Main">
          <ul>
            {TENANT.map((item) => (
              <li key={item.to}>
                <NavLink to={item.to}>{item.label}</NavLink>
              </li>
            ))}
            <li className="shell-scope">
              <label className="shell-namespace">
                <span>Namespace</span>
                <select value={chosen ?? ''} onChange={(event) => namespace.set(event.target.value)} aria-label="Namespace" disabled={items?.length === 0}>
                  {chosen === undefined ? (
                    <option value="" disabled>
                      {items?.length === 0 ? 'No namespaces yet' : 'Choose a namespace'}
                    </option>
                  ) : null}
                  {items === undefined && chosen !== undefined ? <option value={chosen}>{chosen}</option> : null}
                  {(items ?? []).map((item) => (
                    <option key={item.id} value={item.slug}>
                      {item.slug}
                    </option>
                  ))}
                </select>
              </label>
              {chosen !== undefined ? (
                <ul className="shell-subnav" aria-label={`In ${chosen}`}>
                  {SCOPED.map((item) => (
                    <li key={item.to}>
                      <NavLink to={item.to}>{item.label}</NavLink>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="shell-hint">Choose a namespace to open its workflows, runs, operations, scripts and integrations.</p>
              )}
            </li>
          </ul>
        </nav>
      </aside>
      <div className="shell-frame">
        <header className="shell-topbar">
          <div className="shell-crumb" aria-label="Current location">
            {scoped ? (
              <>
                <span>{current ?? 'No namespace'}</span>
                <span aria-hidden="true">/</span>
              </>
            ) : null}
            <strong>{pageName(location.pathname)}</strong>
          </div>
          <span className="shell-session" aria-label="Signed in as">
            <span>{info?.subject}</span>
            {' '}
            <span className="badge" data-tone="idle">{info?.role}</span>
          </span>
          <Button onClick={() => session.signOut('signed-out')}>Sign out</Button>
        </header>
        <main id="main" tabIndex={-1}>
          <Outlet />
        </main>
      </div>
    </div>
  );
}
