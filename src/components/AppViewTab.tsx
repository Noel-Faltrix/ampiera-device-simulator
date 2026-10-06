import { useCallback, useEffect, useState, type FormEvent } from "react";
import * as api from "../api/client";
import type { ChargePointSnapshot, TargetKind } from "../api/types";
import { errorMessage } from "../lib/errors";
import { DASH, formatClock, formatDateTime, formatPower } from "../lib/format";
import { compareBoxWithApp, type CompareState } from "../lib/mismatch";
import { DEFAULT_APP_URL, MIN_NEW_PASSWORD_LENGTH } from "../lib/validation";
import { useSim } from "../state/store";
import { Field } from "./Field";

const POLL_INTERVAL_MS = 30_000;

type ServerChoice = TargetKind | "custom";

function resolveBaseUrl(choice: ServerChoice, custom: string): string {
  return choice === "custom" ? custom.trim().replace(/\/+$/, "") : DEFAULT_APP_URL[choice];
}

function ServerSelect({
  choice,
  custom,
  onChoice,
  onCustom,
}: {
  choice: ServerChoice;
  custom: string;
  onChoice: (c: ServerChoice) => void;
  onCustom: (v: string) => void;
}) {
  return (
    <>
      <Field label="Server">
        {(p) => (
          <select {...p} value={choice} onChange={(e) => onChoice(e.target.value as ServerChoice)}>
            <option value="local">Lokal ({DEFAULT_APP_URL.local})</option>
            <option value="live">Live ({DEFAULT_APP_URL.live})</option>
            <option value="custom">Eigene Adresse</option>
          </select>
        )}
      </Field>
      {choice === "custom" ? (
        <Field label="Adresse des Servers">
          {(p) => (
            <input
              {...p}
              type="text"
              className="mono"
              value={custom}
              onChange={(e) => onCustom(e.target.value)}
              spellCheck={false}
              autoComplete="off"
            />
          )}
        </Field>
      ) : null}
    </>
  );
}

function InviteForm({ choice, custom }: { choice: ServerChoice; custom: string }) {
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const tooShort = password.length < MIN_NEW_PASSWORD_LENGTH;

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (tooShort) return;
    setBusy(true);
    setError(null);
    setDone(false);
    try {
      await api.appRedeemInvite({
        baseUrl: resolveBaseUrl(choice, custom),
        email: email.trim(),
        inviteToken: token.trim(),
        password,
      });
      setPassword("");
      setToken("");
      setDone(true);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <details className="collapsible">
      <summary>Einladung einlösen</summary>
      <form onSubmit={(e) => void submit(e)} noValidate>
        <Field label="E-Mail">
          {(p) => (
            <input
              {...p}
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="off"
            />
          )}
        </Field>
        <Field label="Einladungs-Token">
          {(p) => (
            <input
              {...p}
              type="text"
              className="mono"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
          )}
        </Field>
        <Field
          label="Neues Passwort"
          hint={`${password.length} von mindestens ${MIN_NEW_PASSWORD_LENGTH} Zeichen`}
        >
          {(p) => (
            <input
              {...p}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
            />
          )}
        </Field>
        <button
          type="submit"
          className="btn"
          disabled={busy || tooShort || email === "" || token === ""}
        >
          Einladung einlösen
        </button>
        {done ? (
          <p role="status" className="muted">
            Einladung eingelöst. Du kannst dich jetzt anmelden.
          </p>
        ) : null}
        {error ? (
          <p className="notice notice-critical" role="alert">
            {error}
          </p>
        ) : null}
      </form>
    </details>
  );
}

function LoginPanel({ box }: { box: ChargePointSnapshot }) {
  const { state, dispatch } = useSim();
  const [choice, setChoice] = useState<ServerChoice>(box.config.targetKind);
  const [custom, setCustom] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const needsCode = state.app.auth === "codeRequired";

  async function login(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const result = await api.appLogin({
        baseUrl: resolveBaseUrl(choice, custom),
        email: email.trim(),
        password,
      });
      setPassword("");
      dispatch({
        type: "appAuth",
        auth: result.result === "ok" ? "loggedIn" : "codeRequired",
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function verify(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.appVerifyDevice(code.trim());
      setCode("");
      dispatch({ type: "appAuth", auth: "loggedIn" });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  if (needsCode) {
    return (
      <form className="login" onSubmit={(e) => void verify(e)} noValidate>
        <h3>Anmeldecode eingeben</h3>
        <p className="muted">
          Der Server hat einen Anmeldecode an die E-Mail-Adresse des Testkunden geschickt.
        </p>
        <Field label="Code (6 Ziffern)">
          {(p) => (
            <input
              {...p}
              type="text"
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              className="mono code-input"
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
            />
          )}
        </Field>
        <div className="button-row">
          <button type="submit" className="btn btn-primary" disabled={busy || code.length !== 6}>
            Bestätigen
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => dispatch({ type: "appAuth", auth: "loggedOut" })}
          >
            Zurück
          </button>
        </div>
        {error ? (
          <p className="notice notice-critical" role="alert">
            {error}
          </p>
        ) : null}
      </form>
    );
  }

  return (
    <div className="login-wrap">
      <form className="login" onSubmit={(e) => void login(e)} noValidate>
        <h3>Als Testkunde anmelden</h3>
        <p className="muted">
          Die App-Sicht zeigt, was die Kunden-App vom Server bekommt. Sie liest nur.
        </p>
        <ServerSelect choice={choice} custom={custom} onChoice={setChoice} onCustom={setCustom} />
        <Field label="E-Mail">
          {(p) => (
            <input
              {...p}
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="off"
            />
          )}
        </Field>
        <Field label="Passwort">
          {(p) => (
            <input
              {...p}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          )}
        </Field>
        <button
          type="submit"
          className="btn btn-primary"
          disabled={busy || email === "" || password === ""}
        >
          Anmelden
        </button>
        {error ? (
          <p className="notice notice-critical" role="alert">
            {error}
          </p>
        ) : null}
      </form>
      <InviteForm choice={choice} custom={custom} />
    </div>
  );
}

const STATE_TEXT: Record<CompareState, string> = {
  match: "stimmt überein",
  mismatch: "Abweichung",
  missing: "fehlt in der App",
  info: "",
};

function RawEndpoints({ raw }: { raw: Record<string, unknown> }) {
  const entries = Object.entries(raw);
  if (entries.length === 0)
    return <p className="muted">Der Server hat keine Rohdaten geliefert.</p>;
  return (
    <div className="raw">
      <h4>Rohdaten je Endpunkt</h4>
      {entries.map(([path, body]) => (
        <details key={path} className="collapsible">
          <summary className="mono">{path}</summary>
          <pre className="json">{JSON.stringify(body, null, 2)}</pre>
        </details>
      ))}
    </div>
  );
}

function LoggedInView({ box }: { box: ChargePointSnapshot }) {
  const { state, dispatch } = useSim();
  const snapshot = state.app.snapshot;
  const [busy, setBusy] = useState(false);
  const error = state.app.error;

  const load = useCallback(async () => {
    try {
      dispatch({ type: "appSnapshot", snapshot: await api.appSnapshot() });
    } catch (e) {
      dispatch({ type: "appError", message: errorMessage(e) });
    }
  }, [dispatch]);

  async function refresh() {
    setBusy(true);
    await load();
    setBusy(false);
  }

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, POLL_INTERVAL_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible") void load();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [load]);

  async function logout() {
    try {
      await api.appLogout();
    } catch (e) {
      dispatch({ type: "appError", message: errorMessage(e) });
      return;
    }
    dispatch({ type: "appAuth", auth: "loggedOut" });
  }

  const rows = snapshot ? compareBoxWithApp(box, snapshot.summary) : [];

  return (
    <div className="appview">
      <div className="appview-bar">
        <p className="muted">Nur lesend. Abruf höchstens alle 30 Sekunden.</p>
        <div className="button-row">
          <button type="button" className="btn" disabled={busy} onClick={() => void refresh()}>
            Jetzt abrufen
          </button>
          <button type="button" className="btn" onClick={() => void logout()}>
            Abmelden
          </button>
        </div>
      </div>
      {error ? (
        <p className="notice notice-critical" role="alert">
          {error}
        </p>
      ) : null}
      {snapshot ? (
        <>
          <p className="muted">
            Abgerufen am {formatDateTime(snapshot.fetchedAt)}
            {snapshot.installationId ? (
              <>
                , Anlage <span className="mono">{snapshot.installationId}</span>
              </>
            ) : null}
          </p>
          <table className="compare">
            <thead>
              <tr>
                <th scope="col">Merkmal</th>
                <th scope="col">Box meldet</th>
                <th scope="col">App bekommt</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.key} className={`compare-${row.state}`}>
                  <th scope="row">{row.label}</th>
                  <td>{row.boxValue}</td>
                  <td>
                    <span title={row.note ?? undefined}>{row.appValue}</span>
                    {STATE_TEXT[row.state] ? (
                      <span className={`state-tag state-${row.state}`}>
                        {STATE_TEXT[row.state]}
                      </span>
                    ) : null}
                    {row.note ? <span className="note">{row.note}</span> : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {snapshot.summary.geoHeadline ? (
            <p>
              <span className="muted">Überschrift der Anlagenlage: </span>
              {snapshot.summary.geoHeadline}
            </p>
          ) : null}
          <p>
            <span className="muted">Letzter Kontakt laut App: </span>
            {snapshot.summary.lastContact ? formatDateTime(snapshot.summary.lastContact) : DASH}
          </p>
          <div>
            <h4>Nächste Fahrplanschritte</h4>
            {snapshot.summary.nextScheduleSteps.length === 0 ? (
              <p className="muted">Die App hat keine Fahrplanschritte für diese Anlage.</p>
            ) : (
              <table className="compact">
                <thead>
                  <tr>
                    <th scope="col">Start</th>
                    <th scope="col">Aktion</th>
                    <th scope="col">Zielleistung</th>
                  </tr>
                </thead>
                <tbody>
                  {snapshot.summary.nextScheduleSteps.map((step, i) => (
                    <tr key={`${i}-${step.start}`}>
                      <td>{formatClock(step.start)} Uhr</td>
                      <td>{step.action}</td>
                      <td>{formatPower(step.targetPowerW)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
          <RawEndpoints raw={snapshot.raw} />
        </>
      ) : error === null ? (
        <p className="empty">Die App-Sicht wird abgerufen.</p>
      ) : null}
    </div>
  );
}

export function AppViewTab({ box }: { box: ChargePointSnapshot }) {
  const { state } = useSim();
  return state.app.auth === "loggedIn" ? <LoggedInView box={box} /> : <LoginPanel box={box} />;
}
