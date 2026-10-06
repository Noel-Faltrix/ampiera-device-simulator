import { useState, type FormEvent } from "react";
import * as api from "../api/client";
import type { ChargePointSnapshot } from "../api/types";
import { errorMessage } from "../lib/errors";
import {
  DASH,
  formatCountdown,
  formatMeterWh,
  formatPercent,
  formatPower,
  formatRawLimit,
  formatStand,
} from "../lib/format";
import { parseVehicleForm, type VehicleFormValues } from "../lib/validation";
import { useSim } from "../state/store";
import { Field } from "./Field";
import { useNow } from "./useNow";

const VEHICLE_DEFAULTS: VehicleFormValues = {
  capacityKwh: "60",
  socPct: "30",
  maxPowerKw: "11",
  phases: 3,
};

export function OverviewTab({ box }: { box: ChargePointSnapshot }) {
  const { dispatch } = useSim();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [vehicleForm, setVehicleForm] = useState<VehicleFormValues>(VEHICLE_DEFAULTS);
  const [vehicleError, setVehicleError] = useState<string | null>(null);

  const limit = box.activeLimit;
  const now = useNow(limit?.validTo != null);
  const plugged = box.vehicle?.plugged === true;
  const state = box.connection.state;
  const canConnect = state === "disconnected" || state === "failed";

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function submitVehicle(event: FormEvent) {
    event.preventDefault();
    const parsed = parseVehicleForm(vehicleForm);
    if (!parsed.ok) {
      setVehicleError(parsed.error);
      return;
    }
    setVehicleError(null);
    await run(() => api.plugIn(box.id, parsed.vehicle));
  }

  async function remove() {
    await run(async () => {
      await api.removeChargePoint(box.id);
      dispatch({ type: "removed", id: box.id });
    });
    setConfirmRemove(false);
  }

  return (
    <div className="overview">
      <p className="stand">
        Stand: {formatStand(box.updatedAt)}
        {state !== "connected" ? ". Werte vom letzten Kontakt." : null}
      </p>
      <table className="kv">
        <tbody>
          <tr>
            <th scope="row">OCPP-Status</th>
            <td className="mono">{box.status}</td>
          </tr>
          <tr>
            <th scope="row">Ladeleistung</th>
            <td>{formatPower(box.powerW)}</td>
          </tr>
          <tr>
            <th scope="row">Zählerstand</th>
            <td>{formatMeterWh(box.energyWh)}</td>
          </tr>
          <tr>
            <th scope="row">Transaktions-ID</th>
            <td className="mono">{box.transactionId ?? DASH}</td>
          </tr>
          <tr>
            <th scope="row">Fahrzeug</th>
            <td>
              {plugged && box.vehicle ? (
                <div className="soc">
                  <progress
                    className="soc-bar"
                    aria-label="Ladestand des Fahrzeugs"
                    max={100}
                    value={Math.min(100, Math.max(0, box.vehicle.socPct))}
                  />
                  <span>{formatPercent(box.vehicle.socPct)}</span>
                </div>
              ) : (
                "Kein Fahrzeug angesteckt"
              )}
            </td>
          </tr>
          <tr>
            <th scope="row">Aktive Grenze</th>
            <td>
              {limit ? (
                <div className="limit">
                  <span>
                    {formatPower(limit.limitW)}{" "}
                    <span className="muted">
                      (empfangen: {formatRawLimit(limit.rawLimit, limit.rateUnit)})
                    </span>
                  </span>
                  <span className="muted">
                    Profil <span className="mono">{limit.profileId}</span>,{" "}
                    <span className="mono">{limit.purpose}</span>
                  </span>
                  <span>{formatCountdown(limit.validTo, now)}</span>
                </div>
              ) : (
                "Keine Grenze aktiv, die Wallbox lädt mit ihrem eigenen Maximum."
              )}
            </td>
          </tr>
          <tr>
            <th scope="row">Heartbeat-Intervall</th>
            <td>{box.heartbeatIntervalS === null ? DASH : `${box.heartbeatIntervalS} s`}</td>
          </tr>
          <tr>
            <th scope="row">Letzter Fehler</th>
            <td>{box.lastError ?? DASH}</td>
          </tr>
        </tbody>
      </table>

      <section className="actions" aria-label="Aktionen">
        <h3>Aktionen</h3>
        <div className="button-row">
          <button
            type="button"
            className="btn"
            disabled={busy || !canConnect}
            onClick={() => void run(() => api.connect(box.id))}
          >
            Verbinden
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy || canConnect}
            onClick={() => void run(() => api.disconnect(box.id))}
          >
            Trennen
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy || !plugged}
            onClick={() => void run(() => api.unplug(box.id))}
          >
            Fahrzeug abstecken
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void run(() => api.reboot(box.id))}
          >
            Wallbox neu starten
          </button>
          {confirmRemove ? (
            <span className="confirm" role="group" aria-label="Entfernen bestätigen">
              <span>Wallbox entfernen?</span>
              <button
                type="button"
                className="btn btn-danger"
                disabled={busy}
                onClick={() => void remove()}
              >
                Entfernen
              </button>
              <button type="button" className="btn" onClick={() => setConfirmRemove(false)}>
                Abbrechen
              </button>
            </span>
          ) : (
            <button
              type="button"
              className="btn btn-quiet-danger"
              disabled={busy}
              onClick={() => setConfirmRemove(true)}
            >
              Entfernen
            </button>
          )}
        </div>

        <form className="vehicle-form" onSubmit={(e) => void submitVehicle(e)} noValidate>
          <h4>Fahrzeug anstecken</h4>
          <div className="form-grid form-grid-4">
            <Field label="Akku (kWh)">
              {(p) => (
                <input
                  {...p}
                  type="text"
                  inputMode="decimal"
                  value={vehicleForm.capacityKwh}
                  onChange={(e) => setVehicleForm({ ...vehicleForm, capacityKwh: e.target.value })}
                />
              )}
            </Field>
            <Field label="Ladestand (%)">
              {(p) => (
                <input
                  {...p}
                  type="text"
                  inputMode="decimal"
                  value={vehicleForm.socPct}
                  onChange={(e) => setVehicleForm({ ...vehicleForm, socPct: e.target.value })}
                />
              )}
            </Field>
            <Field label="Max. Ladeleistung (kW)">
              {(p) => (
                <input
                  {...p}
                  type="text"
                  inputMode="decimal"
                  value={vehicleForm.maxPowerKw}
                  onChange={(e) => setVehicleForm({ ...vehicleForm, maxPowerKw: e.target.value })}
                />
              )}
            </Field>
            <Field label="Phasen">
              {(p) => (
                <select
                  {...p}
                  value={vehicleForm.phases}
                  onChange={(e) =>
                    setVehicleForm({ ...vehicleForm, phases: e.target.value === "1" ? 1 : 3 })
                  }
                >
                  <option value="1">1</option>
                  <option value="3">3</option>
                </select>
              )}
            </Field>
          </div>
          {vehicleError ? <p className="field-error">{vehicleError}</p> : null}
          <button type="submit" className="btn btn-primary" disabled={busy || plugged}>
            Fahrzeug anstecken
          </button>
          <p className="field-hint">
            Voreinstellungen eines Standardfahrzeugs. Passe sie bei Bedarf an.
            {plugged ? " Es ist bereits ein Fahrzeug angesteckt." : ""}
          </p>
        </form>

        {error ? (
          <p className="notice notice-critical" role="alert">
            {error}
          </p>
        ) : null}
      </section>
    </div>
  );
}
