import type { AppViewSummary, ChargePointSnapshot } from "../api/types";
import { DASH, formatClock, formatKwh, formatMeterWh, formatPower } from "./format";

export type CompareState = "match" | "mismatch" | "missing" | "info";

export interface CompareRow {
  key: "power" | "status" | "connection" | "quarter";
  label: string;
  boxValue: string;
  appValue: string;
  state: CompareState;
  note: string | null;
}

/** Same tolerance as scenario S11, with a floor so tiny values do not trip on rounding. */
const POWER_TOLERANCE_RATIO = 0.1;
const POWER_TOLERANCE_FLOOR_W = 50;

export function powersAgree(boxW: number, appW: number): boolean {
  const tolerance = Math.max(Math.abs(boxW) * POWER_TOLERANCE_RATIO, POWER_TOLERANCE_FLOOR_W);
  return Math.abs(boxW - appW) <= tolerance;
}

function isOffline(value: string | null): boolean {
  return value !== null && value.trim().toLowerCase() === "offline";
}

export function connectionLabel(box: ChargePointSnapshot): string {
  switch (box.connection.state) {
    case "connected":
      return "Verbunden";
    case "connecting":
      return "Verbindet …";
    case "reconnecting":
      return `Verbindung wird wiederholt (Versuch ${box.connection.attempt})`;
    case "failed":
      return "Fehlgeschlagen";
    case "disconnected":
      return "Getrennt";
  }
}

export function compareBoxWithApp(box: ChargePointSnapshot, app: AppViewSummary): CompareRow[] {
  const boxConnected = box.connection.state === "connected";
  const rows: CompareRow[] = [];

  let powerState: CompareState;
  let powerNote: string | null = null;
  if (app.livePowerW === null) {
    powerState = "missing";
    powerNote =
      "Die App bekommt keine Ladeleistung der Wallbox. Der Server liest sie aus einer Quelle, in die OCPP nicht schreibt.";
  } else if (box.powerW === null) {
    powerState = "info";
    powerNote = "Die Box meldet gerade keine Leistung, ein Vergleich ist nicht möglich.";
  } else if (powersAgree(box.powerW, app.livePowerW)) {
    powerState = "match";
  } else {
    powerState = "mismatch";
    powerNote = "Abweichung größer als 10 %.";
  }
  rows.push({
    key: "power",
    label: "Ladeleistung",
    boxValue: formatPower(box.powerW),
    appValue: formatPower(app.livePowerW),
    state: powerState,
    note: powerNote,
  });

  const appStatus = [app.deviceStatus, app.geoState].filter((v): v is string => v !== null);
  let statusState: CompareState;
  let statusNote: string;
  if (appStatus.length === 0) {
    statusState = "missing";
    statusNote = "Die App bekommt weder Gerätestatus noch Zustand der Wallbox.";
  } else if (boxConnected && (isOffline(app.deviceStatus) || isOffline(app.geoState))) {
    statusState = "mismatch";
    statusNote = "Die Box ist verbunden, die App zeigt sie als nicht erreichbar.";
  } else {
    statusState = "info";
    statusNote = "Die Werte der App haben kein festes Gegenstück im OCPP-Status.";
  }
  rows.push({
    key: "status",
    label: "Status / Zustand",
    boxValue: box.status,
    appValue: appStatus.length > 0 ? appStatus.join(" / ") : DASH,
    state: statusState,
    note: statusNote,
  });

  let connState: CompareState;
  let connNote: string | null = null;
  if (app.connection === null) {
    connState = "missing";
    connNote = "Die App bekommt keinen Verbindungsstatus der Anlage.";
  } else {
    const appOnline = app.connection.trim().toLowerCase() === "online";
    if (boxConnected === appOnline) {
      connState = "match";
    } else {
      connState = "mismatch";
      connNote = boxConnected
        ? "Die Box ist verbunden, die App zeigt die Anlage nicht als online."
        : "Die Box ist getrennt, die App zeigt die Anlage noch als online. Das kann kurz nachlaufen.";
    }
  }
  rows.push({
    key: "connection",
    label: "Verbindung",
    boxValue: connectionLabel(box),
    appValue: app.connection ?? DASH,
    state: connState,
    note: connNote,
  });

  const quarterKnown = app.lastQuarterKwh !== null;
  rows.push({
    key: "quarter",
    label: "Letzte Viertelstunde",
    boxValue: `Zähler ${formatMeterWh(box.energyWh)}`,
    appValue: quarterKnown
      ? `${formatKwh(app.lastQuarterKwh)} um ${formatClock(app.lastQuarterAt)} Uhr`
      : DASH,
    state: quarterKnown ? "info" : "missing",
    note: quarterKnown
      ? "Der Viertelstundenwert ist eine Differenz und nicht direkt mit dem Zählerstand vergleichbar."
      : "Die App hat noch keinen Viertelstundenwert für die Wallbox. Er entsteht alle 15 Minuten.",
  });

  return rows;
}
