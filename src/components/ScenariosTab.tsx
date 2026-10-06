import { useState } from "react";
import * as api from "../api/client";
import type { CheckOutcome, ChargePointSnapshot, ScenarioInfo, ScenarioReport } from "../api/types";
import { errorMessage } from "../lib/errors";
import { formatDateTime, formatElapsed, formatTimeout } from "../lib/format";
import { reportKey, useSim } from "../state/store";
import { useNow } from "./useNow";

const OUTCOME_TEXT: Record<CheckOutcome, string> = {
  passed: "bestanden",
  failed: "nicht bestanden",
  skipped: "übersprungen",
};

const OVERALL_TEXT: Record<ScenarioReport["outcome"], string> = {
  passed: "bestanden",
  failed: "nicht bestanden",
  aborted: "abgebrochen",
};

function OutcomeIcon({ outcome }: { outcome: CheckOutcome }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="16"
      height="16"
      aria-hidden="true"
      focusable="false"
      className="outcome-icon"
    >
      {outcome === "passed" ? (
        <path d="M3 8.5l3.2 3.2L13 5" fill="none" stroke="currentColor" strokeWidth="2" />
      ) : outcome === "failed" ? (
        <path d="M4 4l8 8M12 4l-8 8" fill="none" stroke="currentColor" strokeWidth="2" />
      ) : (
        <path d="M4 8h8" fill="none" stroke="currentColor" strokeWidth="2" />
      )}
    </svg>
  );
}

function ReportView({
  report,
  markdown,
}: {
  report: ScenarioReport;
  markdown: string | undefined;
}) {
  const [feedback, setFeedback] = useState<{ ok: boolean; text: string } | null>(null);

  // The clipboard write must happen inside the click, so the markdown is fetched beforehand.
  function copy() {
    if (markdown === undefined) return;
    navigator.clipboard.writeText(markdown).then(
      () => setFeedback({ ok: true, text: "Bericht kopiert." }),
      (e: unknown) =>
        setFeedback({ ok: false, text: `Kopieren fehlgeschlagen: ${errorMessage(e)}` }),
    );
  }

  async function save() {
    try {
      const path = await api.saveReport(report);
      setFeedback({ ok: true, text: `Gespeichert unter ${path}` });
    } catch (e) {
      setFeedback({ ok: false, text: errorMessage(e) });
    }
  }

  return (
    <div className="report">
      {report.chargePointLabel ? (
        <p className="muted">
          Wallbox: {report.chargePointLabel}
          {report.chargePointIdentity ? (
            <>
              {" "}
              (<span className="mono">{report.chargePointIdentity}</span>)
            </>
          ) : null}
        </p>
      ) : null}
      <div className="report-head">
        <span className={`result result-${report.outcome}`}>
          Ergebnis: {OVERALL_TEXT[report.outcome]}
        </span>
        <span className="muted">
          {formatDateTime(report.startedAt)} bis {formatDateTime(report.finishedAt)}
        </span>
        <button type="button" className="btn" disabled={markdown === undefined} onClick={copy}>
          Bericht kopieren
        </button>
        <button type="button" className="btn" onClick={() => void save()}>
          Bericht speichern
        </button>
        {feedback ? (
          <span role="status" className={feedback.ok ? "muted" : "text-critical"}>
            {feedback.text}
          </span>
        ) : null}
      </div>
      <ul className="checks">
        {report.checks.map((check, index) => (
          <li key={`${index}-${check.name}`} className={`check check-${check.outcome}`}>
            <span className="check-outcome">
              <OutcomeIcon outcome={check.outcome} />
              {OUTCOME_TEXT[check.outcome]}
            </span>
            <span className="check-body">
              <span className="check-name">{check.name}</span>
              <span className="muted">{check.detail}</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

interface ScenarioRowProps {
  box: ChargePointSnapshot;
  scenario: ScenarioInfo;
}

function ScenarioRow({ box, scenario }: ScenarioRowProps) {
  const { state, dispatch } = useSim();
  const [error, setError] = useState<string | null>(null);
  const running = state.running[box.id];
  const isRunning = running?.scenarioId === scenario.id;
  const otherRunning = running !== undefined && !isRunning;
  const isLive = box.config.targetKind === "live";
  const blockedByLive = isLive && !scenario.liveAllowed;
  const report = state.reports[reportKey(box.id, scenario.id)];
  const now = useNow(isRunning);
  const reasonId = `scenario-${scenario.id}-reason`;
  const [announce, setAnnounce] = useState("");
  const blockReason = blockedByLive
    ? "Dieses Szenario läuft nur auf lokalen Wallboxen, weil es auf dem Produktivserver den Betrieb beeinträchtigen könnte."
    : otherRunning
      ? "Auf dieser Wallbox läuft bereits ein Szenario."
      : null;

  async function start() {
    setError(null);
    setAnnounce("Szenario gestartet");
    dispatch({ type: "runStarted", id: box.id, scenarioId: scenario.id, at: Date.now() });
    try {
      const result = await api.runScenario(box.id, scenario.id);
      dispatch({ type: "report", report: result });
      void api.exportReport(result).then(
        (markdown) =>
          dispatch({ type: "reportMarkdown", key: reportKey(box.id, scenario.id), markdown }),
        () => {},
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setAnnounce("Szenario beendet");
      dispatch({ type: "runEnded", id: box.id });
    }
  }

  async function abort() {
    try {
      await api.abortScenario(box.id);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <li className="scenario">
      <div className="scenario-head">
        <div className="scenario-title">
          <span className="mono scenario-id">{scenario.id}</span>
          <h3>{scenario.title}</h3>
          {scenario.needsHuman ? <span className="badge badge-info">Handlung nötig</span> : null}
          {!scenario.liveAllowed ? (
            <span className="badge badge-local-only">nur lokal ausführbar</span>
          ) : null}
          <span className="muted">Zeitlimit {formatTimeout(scenario.timeoutS)}</span>
        </div>
        <div className="button-row">
          {isRunning ? (
            <>
              <span className="muted" aria-hidden="true">
                Läuft seit {formatElapsed((now - (running?.startedAt ?? now)) / 1000)}
              </span>
              <button type="button" className="btn" onClick={() => void abort()}>
                Abbrechen
              </button>
            </>
          ) : (
            <button
              type="button"
              className={isLive ? "btn btn-primary" : "btn"}
              disabled={blockReason !== null}
              aria-describedby={blockReason ? reasonId : undefined}
              onClick={() => void start()}
            >
              {isLive ? "Auf Live starten" : "Starten"}
            </button>
          )}
        </div>
      </div>
      <p className="scenario-desc">{scenario.description}</p>
      <span className="sr-only" role="status">
        {announce}
      </span>
      {blockReason ? (
        <p id={reasonId} className="field-hint">
          {blockReason}
        </p>
      ) : null}
      {scenario.needsHuman ? (
        <p className="field-hint">
          Du musst dafür selbst etwas auslösen. Die Beschreibung sagt, was.
        </p>
      ) : null}
      {error ? (
        <p className="notice notice-critical" role="alert">
          {error}
        </p>
      ) : null}
      {report ? (
        <ReportView
          report={report}
          markdown={state.reportMarkdown[reportKey(box.id, scenario.id)]}
        />
      ) : null}
    </li>
  );
}

export function ScenariosTab({ box }: { box: ChargePointSnapshot }) {
  const { state } = useSim();
  if (state.scenariosError) {
    return (
      <p className="notice notice-critical" role="alert">
        Die Szenarien konnten nicht geladen werden: {state.scenariosError}
      </p>
    );
  }
  if (state.scenarios.length === 0) {
    return <p className="empty">Die Szenarien werden geladen.</p>;
  }
  return (
    <ul className="scenario-list">
      {state.scenarios.map((scenario) => (
        <ScenarioRow key={scenario.id} box={box} scenario={scenario} />
      ))}
    </ul>
  );
}
