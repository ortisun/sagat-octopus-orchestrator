import { Banner, Heading, Stack } from '../../ui';

/** What a page says to someone whose role does not let them see it. Nothing is asked of the platform. */
export function NotAllowed({ what, permission }: { what: string; permission: string }) {
  return (
    <Stack as="section" gap={3}>
      <Heading level={1}>{what}</Heading>
      <Banner tone="warn" title={`You are not allowed to see ${what.toLowerCase()}.`}>
        Your role does not include the {permission} permission. Ask an administrator of your tenant if you need it.
      </Banner>
    </Stack>
  );
}
