import type { ChargePointConfig, TargetKind, VehicleConfig } from "../api/types";

export const IDENTITY_PATTERN = /^[A-Za-z0-9._-]{1,48}$/;
export const DEFAULT_BASE_URL: Record<TargetKind, string> = {
  local: "ws://localhost:9000/ocpp",
  live: "wss://api.ampiera.de/ocpp",
};
export const DEFAULT_APP_URL: Record<TargetKind, string> = {
  local: "http://localhost:3000",
  live: "https://api.ampiera.de",
};
export const MIN_NEW_PASSWORD_LENGTH = 15;

export function isPrivateOrLocalHost(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  if (host === "localhost" || host === "127.0.0.1" || host === "::1") return true;
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (!m) return false;
  const [a, b, c, d] = m.slice(1).map(Number) as [number, number, number, number];
  if ([a, b, c, d].some((n) => n > 255)) return false;
  return a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
}

export function validateBaseUrl(value: string): string | null {
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    return "Das ist keine gültige Adresse. Beispiel: wss://api.ampiera.de/ocpp";
  }
  if (url.protocol === "wss:") return null;
  if (url.protocol !== "ws:") return "Die Adresse muss mit ws:// oder wss:// beginnen.";
  if (!isPrivateOrLocalHost(url.hostname)) {
    return "Unverschlüsseltes ws:// ist nur für localhost und private Adressen erlaubt. Verwende wss://.";
  }
  return null;
}

export interface BoxFormValues {
  label: string;
  targetKind: TargetKind;
  baseUrl: string;
  identity: string;
  password: string;
  vendor: string;
  model: string;
  phases: 1 | 3;
  maxPowerKw: string;
  supportsSoc: boolean;
  unitW: boolean;
  unitA: boolean;
  rejectProfiles: boolean;
  clockOffsetS: string;
}

export type BoxFormErrors = Partial<Record<keyof BoxFormValues, string>>;

export function validateBoxForm(v: BoxFormValues): BoxFormErrors {
  const errors: BoxFormErrors = {};
  if (v.label.trim() === "") errors.label = "Gib der Box eine Bezeichnung.";
  const urlError = validateBaseUrl(v.baseUrl);
  if (urlError) errors.baseUrl = urlError;
  if (!IDENTITY_PATTERN.test(v.identity)) {
    errors.identity =
      "Erlaubt sind 1 bis 48 Zeichen: Buchstaben, Ziffern, Punkt, Unterstrich, Bindestrich.";
  }
  if (v.password === "") errors.password = "Gib das Passwort der Wallbox an.";
  for (const field of ["vendor", "model"] as const) {
    const text = v[field].trim();
    if (text.length === 0) errors[field] = "Darf nicht leer sein.";
    else if (text.length > 20) errors[field] = "Höchstens 20 Zeichen.";
  }
  const kw = parseDecimal(v.maxPowerKw);
  if (kw === null || kw <= 0 || kw > 1000)
    errors.maxPowerKw = "Gib eine Leistung größer als 0 kW an.";
  if (!v.unitW && !v.unitA) errors.unitW = "Wähle mindestens eine Einheit.";
  const offset = Number(v.clockOffsetS);
  if (v.clockOffsetS.trim() === "" || !Number.isInteger(offset)) {
    errors.clockOffsetS = "Gib ganze Sekunden an, 0 für keine Abweichung.";
  }
  return errors;
}

/** Accepts "7,4" and "7.4". */
export function parseDecimal(text: string): number | null {
  const normalized = text.trim().replace(",", ".");
  if (normalized === "") return null;
  const n = Number(normalized);
  return Number.isFinite(n) ? n : null;
}

export function toChargePointConfig(v: BoxFormValues): ChargePointConfig {
  const units: ("W" | "A")[] = [];
  if (v.unitW) units.push("W");
  if (v.unitA) units.push("A");
  return {
    label: v.label.trim(),
    targetKind: v.targetKind,
    baseUrl: v.baseUrl.trim().replace(/\/+$/, ""),
    identity: v.identity,
    vendor: v.vendor.trim(),
    model: v.model.trim(),
    phases: v.phases,
    maxPowerW: Math.round((parseDecimal(v.maxPowerKw) ?? 0) * 1000),
    supportsSoc: v.supportsSoc,
    acceptedRateUnits: units,
    rejectProfiles: v.rejectProfiles,
    clockOffsetS: Number(v.clockOffsetS),
  };
}

export interface VehicleFormValues {
  capacityKwh: string;
  socPct: string;
  maxPowerKw: string;
  phases: 1 | 3;
}

export type VehicleFormResult = { ok: true; vehicle: VehicleConfig } | { ok: false; error: string };

export function parseVehicleForm(v: VehicleFormValues): VehicleFormResult {
  const capacity = parseDecimal(v.capacityKwh);
  const soc = parseDecimal(v.socPct);
  const power = parseDecimal(v.maxPowerKw);
  if (capacity === null || capacity <= 0)
    return { ok: false, error: "Der Akku braucht eine Kapazität größer als 0 kWh." };
  if (soc === null || soc < 0 || soc > 100)
    return { ok: false, error: "Der Ladestand liegt zwischen 0 und 100 %." };
  if (power === null || power <= 0)
    return { ok: false, error: "Die maximale Ladeleistung muss größer als 0 kW sein." };
  return {
    ok: true,
    vehicle: {
      capacityKwh: capacity,
      socPct: soc,
      maxPowerW: Math.round(power * 1000),
      phases: v.phases,
    },
  };
}
