import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface TabDef<T extends string> {
  id: T;
  label: string;
}

interface TabsProps<T extends string> {
  tabs: TabDef<T>[];
  active: T;
  onChange: (id: T) => void;
  idPrefix: string;
  children: ReactNode;
}

export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
  idPrefix,
  children,
}: TabsProps<T>) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});

  function targetIndex(key: string, index: number): number | null {
    if (key === "ArrowRight") return (index + 1) % tabs.length;
    if (key === "ArrowLeft") return (index - 1 + tabs.length) % tabs.length;
    if (key === "Home") return 0;
    if (key === "End") return tabs.length - 1;
    return null;
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const next = targetIndex(
      event.key,
      tabs.findIndex((t) => t.id === active),
    );
    if (next === null) return;
    event.preventDefault();
    const target = tabs[next];
    if (target) {
      onChange(target.id);
      refs.current[target.id]?.focus();
    }
  }

  return (
    <div className="tabs">
      <div
        role="tablist"
        aria-label="Bereiche der Wallbox"
        className="tablist"
        onKeyDown={onKeyDown}
      >
        {tabs.map((tab) => (
          <button
            key={tab.id}
            ref={(el) => {
              refs.current[tab.id] = el;
            }}
            role="tab"
            type="button"
            id={`${idPrefix}-tab-${tab.id}`}
            aria-selected={tab.id === active}
            aria-controls={`${idPrefix}-panel`}
            tabIndex={tab.id === active ? 0 : -1}
            className="tab"
            onClick={() => onChange(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id={`${idPrefix}-panel`}
        aria-labelledby={`${idPrefix}-tab-${active}`}
        className="tabpanel"
      >
        {children}
      </div>
    </div>
  );
}
