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

function ReportView({ report }: { report: ScenarioReport }) {
  const [copyState, setCopyState] = useState<{ ok: boolean; text: string } | null>(null);

  async function copy() {
    try {
      const markdown = await api.exportReport(report);
      await navigator.clipboard.writeText(markdown);
      setCopyState({ ok: true, text: "Bericht kopiert." });
    } catch (e) {
      setCopyState({ ok: false, text: `Kopieren fehlgeschlagen: ${errorMessage(e)}` });
    }
  }

  return (
    <div className="report">
      <div className="report-head">
        <span className={`result result-${report.outcome}`}>
          Ergebnis: {OVERALL_TEXT[report.outcome]}
        </span>
        <span className="muted">
          {formatDateTime(report.startedAt)} bis {formatDateTime(report.finishedAt)}
        </span>
        <button type="button" className="btn" onClick={() => void copy()}>
          Bericht kopieren
        </button>
        {copyState ? (
          <span role="status" className={copyState.ok ? "muted" : "text-critical"}>
            {copyState.text}
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

  async function start() {
    setError(null);
    dispatch({ type: "runStarted", id: box.id, scenarioId: scenario.id, at: Date.now() });
    try {
      const result = await api.runScenario(box.id, scenario.id);
      dispatch({ type: "report", report: result });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
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
          {scenario.needsHuman ? (
            <span className="badge badge-info">braucht Mitwirkung</span>
          ) : null}
          {!scenario.liveAllowed ? <span className="badge badge-local">nur lokal</span> : null}
          <span className="muted">Timeout {formatTimeout(scenario.timeoutS)}</span>
        </div>
        <div className="button-row">
          {isRunning ? (
            <>
              <span role="status" className="muted">
                Läuft seit {formatElapsed((now - (running?.startedAt ?? now)) / 1000)}
              </span>
              <button type="button" className="btn" onClick={() => void abort()}>
                Abbrechen
              </button>
            </>
          ) : (
            <button
              type="button"
              className="btn btn-primary"
              disabled={blockedByLive || otherRunning}
              aria-describedby={blockedByLive ? reasonId : undefined}
              onClick={() => void start()}
            >
              Starten
            </button>
          )}
        </div>
      </div>
      <p className="scenario-desc">{scenario.description}</p>
      {blockedByLive ? (
        <p id={reasonId} className="field-hint">
          Dieses Szenario läuft nur auf lokalen Boxen, weil es auf dem Produktivserver den Betrieb
          beeinträchtigen könnte.
        </p>
      ) : null}
      {scenario.needsHuman && !blockedByLive ? (
        <p className="field-hint">
          Für dieses Szenario musst du selbst etwas auslösen, siehe Beschreibung.
        </p>
      ) : null}
      {error ? (
        <p className="notice notice-critical" role="alert">
          {error}
        </p>
      ) : null}
      {report ? <ReportView report={report} /> : null}
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
