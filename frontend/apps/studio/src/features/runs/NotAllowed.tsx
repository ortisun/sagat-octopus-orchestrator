import { Banner } from '../../ui/Banner';

/** What a page says to someone whose role does not let them see it. Nothing is asked of the platform. */
export function NotAllowed({ what, permission }: { what: string; permission: string }) {
  return (
    <section>
      <h1>{what}</h1>
      <Banner tone="warn" title={`You are not allowed to see ${what.toLowerCase()}.`}>
        Your role does not include the {permission} permission. Ask an administrator of your tenant if you need it.
      </Banner>
    </section>
  );
}
