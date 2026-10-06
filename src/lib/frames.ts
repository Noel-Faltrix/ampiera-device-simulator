import type { FrameLogEntry } from "../api/types";

export type FrameKind = "CALL" | "CALLRESULT" | "CALLERROR" | "unknown";

export interface AnnotatedFrame {
  entry: FrameLogEntry;
  kind: FrameKind;
  /** Action of the call; responses carry the action of the call they answer when it is known. */
  action: string | null;
}

interface Parsed {
  kind: FrameKind;
  uniqueId: string | null;
  callAction: string | null;
}

const parsedCache = new WeakMap<FrameLogEntry, Parsed>();

function parse(entry: FrameLogEntry): Parsed {
  const cached = parsedCache.get(entry);
  if (cached) return cached;
  let result: Parsed = { kind: "unknown", uniqueId: null, callAction: null };
  try {
    const data: unknown = JSON.parse(entry.raw);
    if (Array.isArray(data) && typeof data[1] === "string") {
      const uniqueId = data[1];
      if (data[0] === 2 && typeof data[2] === "string") {
        result = { kind: "CALL", uniqueId, callAction: data[2] };
      } else if (data[0] === 3) {
        result = { kind: "CALLRESULT", uniqueId, callAction: null };
      } else if (data[0] === 4) {
        result = { kind: "CALLERROR", uniqueId, callAction: null };
      }
    }
  } catch {
    // Not JSON: stays "unknown" and is shown as raw text.
  }
  parsedCache.set(entry, result);
  return result;
}

export function annotateFrames(entries: FrameLogEntry[]): AnnotatedFrame[] {
  const actionByCall = new Map<string, string>();
  return entries.map((entry) => {
    const parsed = parse(entry);
    if (parsed.kind === "CALL" && parsed.uniqueId && parsed.callAction) {
      actionByCall.set(parsed.uniqueId, parsed.callAction);
      return { entry, kind: parsed.kind, action: parsed.callAction };
    }
    const action = parsed.uniqueId ? (actionByCall.get(parsed.uniqueId) ?? null) : null;
    return { entry, kind: parsed.kind, action };
  });
}

/** Case-insensitive substring match on the action name; an empty query keeps everything. */
export function filterFrames(frames: AnnotatedFrame[], query: string): AnnotatedFrame[] {
  const q = query.trim().toLowerCase();
  if (q === "") return frames;
  return frames.filter((f) => f.action?.toLowerCase().includes(q) ?? false);
}

export function prettyFrame(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

const keys = new WeakMap<FrameLogEntry, number>();
let nextKey = 0;

/** Stable React key per entry, independent of its position when old entries are dropped. */
export function frameKey(entry: FrameLogEntry): number {
  let key = keys.get(entry);
  if (key === undefined) {
    key = nextKey++;
    keys.set(entry, key);
  }
  return key;
}
