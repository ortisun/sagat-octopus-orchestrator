import { Link as AstryxLink } from '@astryxdesign/core/Link';
import type { AnchorHTMLAttributes, ReactNode, Ref } from 'react';
import { Link as RouterLink } from 'react-router';

/** How every Astryx link navigates inside the Studio: through the router, so the page is not reloaded. */
export function RoutedAnchor({ href, ref, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement> & { ref?: Ref<HTMLAnchorElement> }) {
  if (href === undefined || /^[a-z][a-z0-9+.-]*:/i.test(href) || rest.target === '_blank') {
    // eslint-disable-next-line jsx-a11y/anchor-has-content -- the content arrives in the props Astryx passes through
    return <a href={href} ref={ref} {...rest} />;
  }
  return <RouterLink to={href} ref={ref} {...rest} />;
}

/** A link to a page of the Studio (`to`) or, with `external`, to another site in a new tab. */
export function Link({ to, children, external, 'aria-label': label, className }: { to: string; children: ReactNode; external?: boolean; 'aria-label'?: string; className?: string }) {
  return (
    <AstryxLink as={RoutedAnchor} href={to} type="inherit" hasUnderline isExternalLink={external === true} {...(label ? { label } : {})} {...(className ? { className } : {})}>
      {children}
    </AstryxLink>
  );
}
