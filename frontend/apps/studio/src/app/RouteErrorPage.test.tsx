import { render, screen } from '@testing-library/react';
import { createMemoryRouter, Outlet, RouterProvider } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { routes } from './routes';

afterEach(() => vi.restoreAllMocks());

describe('route failure recovery', () => {
  it.each(['lazy import', 'render'])('offers recovery for a failed %s without exposing exception details', async (failure) => {
    // These are deliberately thrown into the real router, which reports them to the console.
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const error = new Error('Unable to preload CSS for /studio/assets/private-canary.css');
    function BrokenPage(): never { throw error; }
    const router = createMemoryRouter([{
      path: '/',
      errorElement: routes[0]?.errorElement,
      element: <Outlet />,
      hydrateFallbackElement: <p>Loading</p>,
      children: [{
        path: 'workflows',
        ...(failure === 'lazy import'
          ? { lazy: () => Promise.reject(error) }
          : { Component: BrokenPage }),
      }],
    }], { initialEntries: ['/workflows'] });
    render(<RouterProvider router={router} />);

    const heading = await screen.findByRole('heading', { name: 'Unable to open this page' });
    expect(heading).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Reload Studio' })).toBeVisible();
    expect(screen.getByText(/unsaved changes will be lost/i)).toBeVisible();
    expect(screen.getByText(/sign in again/i)).toBeVisible();
    expect(screen.queryByText(/private-canary|Unexpected Application Error|Hey developer/)).not.toBeInTheDocument();
    router.dispose();
  });
});
