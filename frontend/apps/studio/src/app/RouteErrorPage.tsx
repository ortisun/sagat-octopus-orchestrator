import { useEffect, useRef } from 'react';
import { Button, FocusHeading, Stack, Text } from '../ui';

/** Included in the entry bundle so a missing route chunk cannot also remove its recovery UI. */
export function RouteErrorPage() {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { heading.current?.focus(); }, []);

  return (
    <main className="route-error" aria-labelledby="route-error-title">
      <Stack gap={3}>
        <FocusHeading level={1} id="route-error-title" ref={heading}>Unable to open this page</FocusHeading>
        <Text>The Studio could not display this page. Reload to try again.</Text>
        <Text>Unsaved changes will be lost, and you will need to sign in again.</Text>
        <div>
          <Button variant="primary" onClick={() => window.location.reload()}>Reload Studio</Button>
        </div>
      </Stack>
    </main>
  );
}
