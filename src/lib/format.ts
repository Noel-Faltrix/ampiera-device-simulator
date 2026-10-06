const NUMBER_DE = new Intl.NumberFormat("de-DE", { maximumFractionDigits: 2 });
const NUMBER_DE_1 = new Intl.NumberFormat("de-DE", { maximumFractionDigits: 1 });
const NUMBER_DE_0 = new Intl.NumberFormat("de-DE", { maximumFractionDigits: 0 });

export const DASH = "—";

/** Watts below 1000 stay in W, everything above is shown in kW. */
export function formatPower(watts: number | null): string {
  if (watts === null || !Number.isFinite(watts)) return DASH;
  if (Math.abs(watts) >= 1000) return `${NUMBER_DE.format(watts / 1000)} kW`;
  return `${NUMBER_DE_0.format(watts)} W`;
}

export function formatKwh(kwh: number | null): string {
  if (kwh === null || !Number.isFinite(kwh)) return DASH;
  return `${NUMBER_DE.format(kwh)} kWh`;
}

export function formatMeterWh(wh: number | null): string {
  return formatKwh(wh === null ? null : wh / 1000);
}

export function formatPercent(pct: number | null): string {
  if (pct === null || !Number.isFinite(pct)) return DASH;
  return `${NUMBER_DE_1.format(pct)} %`;
}

export function formatRawLimit(raw: number, unit: "W" | "A"): string {
  return `${NUMBER_DE_1.format(raw)} ${unit}`;
}

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** "endet in 4:32 min", "endet in 1:04:32 h", "abgelaufen" or "ohne Ablaufzeit". */
export function formatCountdown(validTo: string | null, nowMs: number): string {
  if (validTo === null) return "ohne Ablaufzeit";
  const target = Date.parse(validTo);
  if (Number.isNaN(target)) return DASH;
  const remaining = Math.floor((target - nowMs) / 1000);
  if (remaining <= 0) return "abgelaufen";
  const hours = Math.floor(remaining / 3600);
  const minutes = Math.floor((remaining % 3600) / 60);
  const seconds = remaining % 60;
  if (hours > 0) return `endet in ${hours}:${pad(minutes)}:${pad(seconds)} h`;
  return `endet in ${minutes}:${pad(seconds)} min`;
}

/** Local time hh:mm:ss.SSS, used in the protocol list. */
export function formatTimeMs(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return DASH;
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export function formatClock(iso: string | null): string {
  if (iso === null) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return DASH;
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function formatDateTime(iso: string | null): string {
  if (iso === null) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return DASH;
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatTimeout(seconds: number): string {
  if (seconds < 120) return `${seconds} s`;
  return `${NUMBER_DE_0.format(Math.round(seconds / 60))} min`;
}

export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  return `${Math.floor(s / 60)}:${pad(s % 60)} min`;
}
