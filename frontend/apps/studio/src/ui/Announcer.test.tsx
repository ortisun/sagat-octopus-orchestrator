import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AnnouncerProvider, useAnnounce } from './Announcer';

function Speaker({ say }: { say: string[] }) {
  const announce = useAnnounce();
  return (
    <button type="button" onClick={() => say.forEach(announce)}>
      say
    </button>
  );
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('AnnouncerProvider', () => {
  it('announces every message that comes together, in order, and the same text twice', () => {
    render(
      <AnnouncerProvider>
        <Speaker say={['Saved', 'Saved', '2 problems']} />
      </AnnouncerProvider>,
    );
    act(() => screen.getByRole('button', { name: 'say' }).click());
    const region = screen.getByRole('status');
    expect([...region.querySelectorAll('p')].map((p) => p.textContent)).toEqual(['Saved', 'Saved', '2 problems']);
    expect(region).toHaveAttribute('aria-live', 'polite');
    expect(region).toHaveAttribute('aria-atomic', 'false');
  });

  it('lets a message go after a couple of minutes, and keeps only the last few', () => {
    render(
      <AnnouncerProvider>
        <Speaker say={['1', '2', '3', '4', '5', '6', '7']} />
      </AnnouncerProvider>,
    );
    act(() => screen.getByRole('button', { name: 'say' }).click());
    const texts = () => [...screen.getByRole('status').querySelectorAll('p')].map((p) => p.textContent);
    expect(texts()).toEqual(['3', '4', '5', '6', '7']);
    act(() => void vi.advanceTimersByTime(121_000));
    expect(texts()).toEqual([]);
  });
});
