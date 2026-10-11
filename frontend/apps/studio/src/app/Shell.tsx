import { useEffect } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router';
import { Button } from '../ui/Button';
import { useApp, useNamespace, useSession } from './context';
import { useQuery } from '@tanstack/react-query';

const NAV = [
  { to: '/workflows', label: 'Workflows' },
  { to: '/runs', label: 'Runs' },
  { to: '/scripts', label: 'Scripts' },
] as const;

/** The frame of every page: skip link, header with the namespace and the session, navigation, main. */
export function Shell() {
  const { session, ep, namespace } = useApp();
  const { info } = useSession();
  const current = useNamespace();
  const location = useLocation();
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });

  // With one namespace there is nothing to choose; with several, the first until the user picks.
  useEffect(() => {
    const items = namespaces.data?.items ?? [];
    if (items.length > 0 && (current === undefined || !items.some((n) => n.slug === current))) {
      namespace.set(items[0]?.slug);
    }
  }, [namespaces.data, current, namespace]);

  // After a navigation focus moves to the page, so a keyboard or screen-reader user starts at its top.
  useEffect(() => {
    document.getElementById('main')?.focus({ preventScroll: true });
  }, [location.pathname]);

  return (
    <div className="shell">
      <a className="skip-link" href="#main">
        Skip to the content
      </a>
      <header>
        <span className="brand">Sagat Studio</span>
        <label>
          <span className="visually-hidden">Namespace</span>
          <select className="input" value={current ?? ''} onChange={(event) => namespace.set(event.target.value)} aria-label="Namespace" disabled={(namespaces.data?.items.length ?? 0) === 0}>
            {(namespaces.data?.items ?? []).map((item) => (
              <option key={item.id} value={item.slug}>
                {item.slug}
              </option>
            ))}
          </select>
        </label>
        <span aria-label="Signed in as">
          {info?.subject} <span className="badge" data-tone="idle">{info?.role}</span>
        </span>
        <Button onClick={() => session.signOut('signed-out')}>Sign out</Button>
      </header>
      <nav aria-label="Main">
        <ul>
          {NAV.map((item) => (
            <li key={item.to}>
              <NavLink to={item.to}>{item.label}</NavLink>
            </li>
          ))}
        </ul>
      </nav>
      <main id="main" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}
