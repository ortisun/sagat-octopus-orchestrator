import { useEffect, useRef } from 'react';
import { Button } from '../ui/Button';

/** Included in the entry bundle so a missing route chunk cannot also remove its recovery UI. */
export function RouteErrorPage() {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { heading.current?.focus(); }, []);

  return (
    <main className="route-error" aria-labelledby="route-error-title">
      <h1 id="route-error-title" ref={heading} tabIndex={-1}>Unable to open this page</h1>
      <p>The Studio could not display this page. Reload to try again.</p>
      <p>Unsaved changes will be lost, and you will need to sign in again.</p>
      <Button variant="primary" onClick={() => window.location.reload()}>Reload Studio</Button>
    </main>
  );
}
