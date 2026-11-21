import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { announcedAssertively, expectAvailable, expectUnavailable } from '../../test/ui';
import { createApp } from './context';
import { SignIn } from './SignIn';

describe('SignIn', () => {
  it('signs in with a token the platform accepts and keeps it in memory only', async () => {
    const platform = fakePlatform({ 'GET /auth/context': () => AUTH_CONTEXT });
    const { app } = renderApp(<SignIn />, { appOptions: { fetch: platform.fetcher } });
    const user = userEvent.setup();
    const button = screen.getByRole('button', { name: 'Sign in' });
    expectUnavailable(button);
    await user.click(screen.getByRole('textbox', { name: /^Access token/ }));
    await user.paste(TOKEN);
    expectAvailable(button);
    await user.click(button);
    await waitFor(() => expect(app.session.token()).toBe(TOKEN));
    expect(app.session.snapshot().info?.subject).toBe('alice');
    const sent = platform.calls[0]!;
    expect((sent.init.headers as Record<string, string>)['Authorization']).toBe(`Bearer ${TOKEN}`);
    expect(sent.url).toBe('/v1/auth/context');
    // The field is emptied: the token is not left in the page.
    expect(screen.getByRole('textbox', { name: /^Access token/ })).toHaveValue('');
  });

  it('says so when the platform refuses the token, and holds nothing', async () => {
    const platform = fakePlatform({
      'GET /auth/context': () => new Response(JSON.stringify({ code: 'UNAUTHENTICATED', message: 'the token has expired' }), { status: 401, headers: { 'content-type': 'application/json' } }),
    });
    const { app } = renderApp(<SignIn />, { appOptions: { fetch: platform.fetcher } });
    const user = userEvent.setup();
    await user.click(screen.getByRole('textbox', { name: /^Access token/ }));
    await user.paste(TOKEN);
    await user.click(screen.getByRole('button', { name: 'Sign in' }));
    // The refusal is the field's error: tied to the field, marking it invalid, and announced.
    const field = screen.getByRole('textbox', { name: /^Access token/ });
    await waitFor(() => expect(field).toHaveAccessibleDescription(expect.stringContaining('the token has expired')));
    expect(field).toBeInvalid();
    await waitFor(() => expect(announcedAssertively()).toContain('the token has expired'));
    expect(app.session.token()).toBeUndefined();
  });

  it('does not send something that is not a token', async () => {
    const platform = fakePlatform({});
    renderApp(<SignIn />, { appOptions: { fetch: platform.fetcher } });
    const user = userEvent.setup();
    await user.click(screen.getByRole('textbox', { name: /^Access token/ }));
    await user.paste('hello world');
    expectUnavailable(screen.getByRole('button', { name: 'Sign in' }));
    expect(platform.calls).toHaveLength(0);
  });

  it('tells a user whose session ended why', async () => {
    const platform = fakePlatform({ 'GET /auth/context': () => AUTH_CONTEXT });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
    app.session.signOut('expired');
    renderApp(<SignIn />, { app });
    expect(await screen.findByText(/token expired/)).toBeInTheDocument();
  });
});
