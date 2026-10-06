import type { ChargePointSnapshot, TargetKind } from "../api/types";
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
