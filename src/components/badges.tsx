import type { ChargePointSnapshot, FrameDirection, TargetKind } from "../api/types";
import { connectionLabel } from "../lib/mismatch";

export function TargetBadge({ kind }: { kind: TargetKind }) {
  return kind === "live" ? (
    <span className="badge badge-live">Live</span>
  ) : (
    <span className="badge badge-local">Lokal</span>
  );
}

function dotClass(box: ChargePointSnapshot): string {
  switch (box.connection.state) {
    case "connected":
      return "dot-good";
    case "connecting":
    case "reconnecting":
      return "dot-warning";
    case "failed":
      return "dot-critical";
    case "disconnected":
      return "dot-muted";
  }
}

export function ConnectionIndicator({ box }: { box: ChargePointSnapshot }) {
  return (
    <span className="conn">
      <span className={`dot ${dotClass(box)}`} aria-hidden="true" />
      <span>{connectionLabel(box)}</span>
    </span>
  );
}

/** Plex Latin has no arrow glyphs, so the direction is drawn as SVG. */
export function DirectionLabel({ direction }: { direction: FrameDirection }) {
  const out = direction === "out";
  return (
    <span className="direction">
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
        <path
          d={out ? "M2 8h11M9 4l4 4-4 4" : "M14 8H3M7 4L3 8l4 4"}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
        />
      </svg>
      <span className="sr-only">{out ? "an die Zentrale" : "von der Zentrale"}</span>
      <span aria-hidden="true">Zentrale</span>
    </span>
  );
}
