import { useLayoutEffect, useMemo, useRef, useState } from "react";
import * as api from "../api/client";
import type { ChargePointSnapshot } from "../api/types";
import { DirectionLabel } from "./badges";
import { FRAME_CAP } from "../lib/constants";
import { errorMessage } from "../lib/errors";
import { formatTimeMs } from "../lib/format";
import { annotateFrames, filterFrames, frameKey, prettyFrame } from "../lib/frames";
import { useSim } from "../state/store";

const STICK_THRESHOLD_PX = 24;
const NO_FRAMES: never[] = [];

export function LogTab({ box }: { box: ChargePointSnapshot }) {
  const { state } = useSim();
  const frames = state.frames[box.id] ?? NO_FRAMES;
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<number | null>(null);
  const [exportNote, setExportNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [stuck, setStuck] = useState(true);
  const scroller = useRef<HTMLDivElement>(null);

  const annotated = useMemo(() => annotateFrames(frames), [frames]);
  const visible = useMemo(() => filterFrames(annotated, query), [annotated, query]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stuck) el.scrollTop = el.scrollHeight;
  }, [visible.length, stuck]);

  function onScroll() {
    const el = scroller.current;
    if (!el) return;
    setStuck(el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_THRESHOLD_PX);
  }

  async function exportLog() {
    setExportNote(null);
    try {
      const path = await api.saveLog(box.id);
      setExportNote({ ok: true, text: `Gespeichert unter ${path}` });
    } catch (e) {
      setExportNote({ ok: false, text: errorMessage(e) });
    }
  }

  return (
    <div className="log">
      <div className="log-bar">
        <div className="field log-filter">
          <label htmlFor="log-filter">Filter nach Aktion</label>
          <input
            id="log-filter"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="z. B. MeterValues"
            autoComplete="off"
          />
        </div>
        <span className="muted" aria-live="polite">
          {visible.length} von {frames.length} Einträgen
          {frames.length >= FRAME_CAP ? " (gekürzt)" : ""}
        </span>
        <button type="button" className="btn" onClick={() => void exportLog()}>
          Exportieren
        </button>
      </div>
      {exportNote ? (
        <p
          role={exportNote.ok ? "status" : "alert"}
          className={exportNote.ok ? "muted" : "notice notice-critical"}
        >
          {exportNote.text}
        </p>
      ) : null}
      {frames.length === 0 ? (
        <p className="empty">
          Noch kein Datenverkehr. Sobald die Wallbox sich mit der Zentrale verbindet, erscheinen
          hier die letzten {FRAME_CAP} OCPP-Nachrichten.
        </p>
      ) : visible.length === 0 ? (
        <p className="empty">Keine Nachricht passt zum Filter.</p>
      ) : (
        <div
          className="log-scroll"
          ref={scroller}
          onScroll={onScroll}
          tabIndex={0}
          role="region"
          aria-label="OCPP-Nachrichten"
        >
          <div className="frame-head" aria-hidden="true">
            <span>Zeit (Ortszeit)</span>
            <span>Richtung</span>
            <span>Aktion</span>
            <span>Nachricht</span>
          </div>
          <ul className="frames">
            {visible.map(({ entry, kind, action }) => {
              const key = frameKey(entry);
              const open = expanded === key;
              return (
                <li key={key} className={`frame frame-${entry.direction}`}>
                  <button
                    type="button"
                    className="frame-line"
                    aria-expanded={open}
                    onClick={() => setExpanded(open ? null : key)}
                  >
                    <span className="mono frame-time">{formatTimeMs(entry.at)}</span>
                    <span className="frame-dir">
                      <DirectionLabel direction={entry.direction} />
                    </span>
                    <span className="mono frame-action">
                      {action ?? "?"}
                      {kind === "CALLRESULT"
                        ? " (Antwort)"
                        : kind === "CALLERROR"
                          ? " (Fehler)"
                          : ""}
                    </span>
                    <span className="mono frame-raw">{entry.raw}</span>
                  </button>
                  {open ? <pre className="json">{prettyFrame(entry.raw)}</pre> : null}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {!stuck && frames.length > 0 ? (
        <button
          type="button"
          className="btn jump"
          onClick={() => {
            setStuck(true);
          }}
        >
          Zum Ende springen
        </button>
      ) : null}
    </div>
  );
}
