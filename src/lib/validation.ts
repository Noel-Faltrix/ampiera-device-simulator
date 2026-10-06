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

export const LIMITS = {
  labelMax: 60,
  vendorModelMax: 20,
  passwordMax: 200,
  maxPowerKwMax: 350,
  capacityKwhMax: 300,
  clockOffsetMaxS: 86400,
};

function hasControlChars(text: string): boolean {
  return [...text].some((ch) => {
    const code = ch.charCodeAt(0);
    return code < 0x20 || code === 0x7f;
  });
}
const PRINTABLE_ASCII = /^[\x20-\x7e]+$/;

/** True when the address points to a host outside the local network, which the core treats as live. */
export function isPublicUrl(value: string): boolean {
  try {
    return !isPrivateOrLocalHost(new URL(value.trim()).hostname);
  } catch {
    return false;
  }
}

export function validateBaseUrl(value: string): string | null {
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    return "Das ist keine gültige Adresse. Beispiel: wss://api.ampiera.de/ocpp";
  }
  if (url.username || url.password || url.search || url.hash) {
    return "Die Adresse darf keine Zugangsdaten, Parameter oder Anker enthalten.";
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
  const label = v.label.trim();
  if (label === "") errors.label = "Gib der Wallbox eine Bezeichnung.";
  else if (label.length > LIMITS.labelMax) errors.label = `Höchstens ${LIMITS.labelMax} Zeichen.`;
  else if (hasControlChars(label)) errors.label = "Keine Steuerzeichen erlaubt.";
  const urlError = validateBaseUrl(v.baseUrl);
  if (urlError) errors.baseUrl = urlError;
  else if (v.targetKind === "local" && isPublicUrl(v.baseUrl)) {
    errors.baseUrl =
      "Diese Adresse liegt nicht im lokalen Netz. Wähle das Ziel Live oder eine lokale Adresse.";
  }
  if (!IDENTITY_PATTERN.test(v.identity)) {
    errors.identity =
      "Erlaubt sind 1 bis 48 Zeichen: Buchstaben, Ziffern, Punkt, Unterstrich, Bindestrich.";
  }
  if (v.password === "") errors.password = "Gib das Passwort der Wallbox an.";
  else if (v.password.length > LIMITS.passwordMax || !PRINTABLE_ASCII.test(v.password)) {
    errors.password = `1 bis ${LIMITS.passwordMax} druckbare ASCII-Zeichen, ohne Umlaute.`;
  }
  for (const field of ["vendor", "model"] as const) {
    const text = v[field].trim();
    if (text.length === 0) errors[field] = "Darf nicht leer sein.";
    else if (text.length > LIMITS.vendorModelMax) {
      errors[field] = `Höchstens ${LIMITS.vendorModelMax} Zeichen.`;
    } else if (hasControlChars(text)) errors[field] = "Keine Steuerzeichen erlaubt.";
  }
  const kw = parseDecimal(v.maxPowerKw);
  if (kw === null || kw < 0.001 || kw > LIMITS.maxPowerKwMax) {
    errors.maxPowerKw = `Gib eine Leistung von 0,001 bis ${LIMITS.maxPowerKwMax} kW an.`;
  }
  if (!v.unitW && !v.unitA) errors.unitW = "Wähle mindestens eine Einheit.";
  const offset = Number(v.clockOffsetS);
  if (
    v.clockOffsetS.trim() === "" ||
    !Number.isInteger(offset) ||
    Math.abs(offset) > LIMITS.clockOffsetMaxS
  ) {
    errors.clockOffsetS = `Gib ganze Sekunden zwischen -${LIMITS.clockOffsetMaxS} und ${LIMITS.clockOffsetMaxS} an, 0 für keine Abweichung.`;
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
  if (capacity === null || capacity < 1 || capacity > LIMITS.capacityKwhMax)
    return {
      ok: false,
      error: `Die Akkukapazität liegt zwischen 1 und ${LIMITS.capacityKwhMax} kWh.`,
    };
  if (soc === null || soc < 0 || soc > 100)
    return { ok: false, error: "Der Ladestand liegt zwischen 0 und 100 %." };
  if (power === null || power < 0.001 || power > LIMITS.maxPowerKwMax)
    return {
      ok: false,
      error: `Die maximale Ladeleistung liegt zwischen 0,001 und ${LIMITS.maxPowerKwMax} kW.`,
    };
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
