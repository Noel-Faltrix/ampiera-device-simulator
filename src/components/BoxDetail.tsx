import { useState } from "react";
import type { ChargePointSnapshot } from "../api/types";
import { AppViewTab } from "./AppViewTab";
import { ConnectionIndicator, TargetBadge } from "./badges";
import { LogTab } from "./LogTab";
import { OverviewTab } from "./OverviewTab";
import { ScenariosTab } from "./ScenariosTab";
import { Tabs, type TabDef } from "./Tabs";

type TabId = "overview" | "scenarios" | "app" | "log";

const TABS: TabDef<TabId>[] = [
  { id: "overview", label: "Übersicht" },
  { id: "scenarios", label: "Szenarien" },
  { id: "app", label: "App-Sicht" },
  { id: "log", label: "Protokoll" },
];

export function BoxDetail({ box }: { box: ChargePointSnapshot }) {
  const [tab, setTab] = useState<TabId>("overview");
  return (
    <section className="detail" aria-label={`Wallbox ${box.config.label}`}>
      <div className={`detail-head${box.config.targetKind === "live" ? " detail-head-live" : ""}`}>
        <h2>{box.config.label}</h2>
        <TargetBadge kind={box.config.targetKind} />
        <span className="mono">{box.config.identity}</span>
        <ConnectionIndicator box={box} />
        <span className="mono muted detail-url">{box.config.baseUrl}</span>
      </div>
      {box.connection.state === "failed" ? (
        <p className="notice notice-critical" role="alert">
          Verbindung fehlgeschlagen: {box.connection.reason}
        </p>
      ) : null}
      <Tabs tabs={TABS} active={tab} onChange={setTab} idPrefix="box">
        {tab === "overview" ? <OverviewTab key={box.id} box={box} /> : null}
        {tab === "scenarios" ? <ScenariosTab box={box} /> : null}
        {tab === "app" ? <AppViewTab box={box} /> : null}
        {tab === "log" ? <LogTab key={box.id} box={box} /> : null}
      </Tabs>
    </section>
  );
}
