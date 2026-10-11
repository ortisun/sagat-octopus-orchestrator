import { useId, useRef } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';

export interface TabSpec {
  id: string;
  label: string;
  panel: ReactNode;
}

/** Tabs with the keyboard model of the ARIA pattern: arrows move, Home and End jump, Tab leaves. */
export function Tabs({ tabs, selected, onSelect, label }: { tabs: TabSpec[]; selected: string; onSelect: (id: string) => void; label: string }) {
  const base = useId();
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const index = Math.max(
    0,
    tabs.findIndex((t) => t.id === selected),
  );
  const move = (event: KeyboardEvent, to: number) => {
    event.preventDefault();
    const next = tabs[(to + tabs.length) % tabs.length];
    if (next) {
      onSelect(next.id);
      refs.current.get(next.id)?.focus();
    }
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'ArrowRight') move(event, index + 1);
    else if (event.key === 'ArrowLeft') move(event, index - 1);
    else if (event.key === 'Home') move(event, 0);
    else if (event.key === 'End') move(event, tabs.length - 1);
  };
  const current = tabs[index];
  return (
    <div>
      <div role="tablist" aria-label={label} className="toolbar">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            ref={(element) => {
              if (element) refs.current.set(tab.id, element);
              else refs.current.delete(tab.id);
            }}
            role="tab"
            type="button"
            id={`${base}-tab-${tab.id}`}
            aria-selected={tab.id === current?.id}
            aria-controls={`${base}-panel-${tab.id}`}
            tabIndex={tab.id === current?.id ? 0 : -1}
            className="btn"
            onClick={() => onSelect(tab.id)}
            onKeyDown={onKeyDown}
          >
            {tab.label}
          </button>
        ))}
      </div>
      {current ? (
        <div role="tabpanel" id={`${base}-panel-${current.id}`} aria-labelledby={`${base}-tab-${current.id}`} tabIndex={0}>
          {current.panel}
        </div>
      ) : null}
    </div>
  );
}
