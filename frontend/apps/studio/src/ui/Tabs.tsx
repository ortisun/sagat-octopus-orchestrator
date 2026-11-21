import { Tab, TabList } from '@astryxdesign/core/TabList';
import { useId } from 'react';
import type { ReactNode } from 'react';

export interface TabSpec {
  id: string;
  label: string;
  panel: ReactNode;
}

/** Tabs that switch a panel in place, with the keyboard model of the ARIA pattern (Astryx `TabList` in its `tablist` role). */
export function Tabs({ tabs, selected, onSelect, label }: { tabs: TabSpec[]; selected: string; onSelect: (id: string) => void; label: string }) {
  const base = useId();
  const current = tabs.find((tab) => tab.id === selected) ?? tabs[0];
  return (
    <div>
      <TabList role="tablist" aria-label={label} value={current?.id ?? ''} onChange={onSelect} hasDivider>
        {tabs.map((tab) => (
          <Tab key={tab.id} id={`${base}-tab-${tab.id}`} value={tab.id} label={tab.label} panelId={`${base}-panel-${tab.id}`} />
        ))}
      </TabList>
      {current ? (
        <div role="tabpanel" id={`${base}-panel-${current.id}`} aria-labelledby={`${base}-tab-${current.id}`} tabIndex={0}>
          {current.panel}
        </div>
      ) : null}
    </div>
  );
}
