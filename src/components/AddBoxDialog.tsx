import { useEffect, useRef, useState, type FormEvent } from "react";
import * as api from "../api/client";
import type { TargetKind } from "../api/types";
import { LIVE_BOX_HINT } from "../lib/constants";
import { errorMessage } from "../lib/errors";
import {
  DEFAULT_BASE_URL,
  toChargePointConfig,
  validateBoxForm,
  type BoxFormErrors,
  type BoxFormValues,
} from "../lib/validation";
import { Field } from "./Field";

interface AddBoxDialogProps {
  onClose: () => void;
  onAdded: (id: string) => void;
}

const INITIAL: BoxFormValues = {
  label: "",
  targetKind: "local",
  baseUrl: DEFAULT_BASE_URL.local,
  identity: "",
  password: "",
  vendor: "Ampiera Sim",
  model: "Device Simulator",
  phases: 3,
  maxPowerKw: "11",
  supportsSoc: true,
  unitW: true,
  unitA: true,
  rejectProfiles: false,
  clockOffsetS: "0",
};

export function AddBoxDialog({ onClose, onAdded }: AddBoxDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [values, setValues] = useState<BoxFormValues>(INITIAL);
  const [urlEdited, setUrlEdited] = useState(false);
  const [liveConfirmed, setLiveConfirmed] = useState(false);
  const [errors, setErrors] = useState<BoxFormErrors>({});
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const opener = useRef<Element | null>(document.activeElement);

  useEffect(() => {
    if (attempt === 0) return;
    dialogRef.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
  }, [attempt]);

  function close() {
    const dialog = dialogRef.current;
    if (dialog?.open && typeof dialog.close === "function") dialog.close();
    onClose();
    if (opener.current instanceof HTMLElement) opener.current.focus();
  }

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (typeof dialog.showModal === "function") {
      if (!dialog.open) dialog.showModal();
    } else {
      dialog.setAttribute("open", "");
    }
  }, []);

  const isLive = values.targetKind === "live";

  function set<K extends keyof BoxFormValues>(key: K, value: BoxFormValues[K]) {
    setValues((prev) => ({ ...prev, [key]: value }));
  }

  function chooseTarget(kind: TargetKind) {
    setValues((prev) => ({
      ...prev,
      targetKind: kind,
      baseUrl: urlEdited ? prev.baseUrl : DEFAULT_BASE_URL[kind],
    }));
    if (kind === "local") setLiveConfirmed(false);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (submitting) return;
    const found = validateBoxForm(values);
    setErrors(found);
    if (Object.keys(found).length > 0) {
      setAttempt((n) => n + 1);
      return;
    }
    if (isLive && !liveConfirmed) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const id = await api.addChargePoint({
        config: toChargePointConfig(values),
        password: values.password,
        liveConfirmed: isLive && liveConfirmed,
      });
      onAdded(id);
      close();
    } catch (error) {
      setSubmitError(errorMessage(error));
      setSubmitting(false);
    }
  }

  return (
    <dialog
      ref={dialogRef}
      className={`dialog${isLive ? " dialog-live" : ""}`}
      aria-labelledby="add-box-title"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <form onSubmit={(e) => void submit(e)} noValidate>
        <h2 id="add-box-title">{isLive ? "Live-Wallbox hinzufügen" : "Wallbox hinzufügen"}</h2>
        <p className="field-hint">Mit * markierte Felder sind Pflichtfelder.</p>

        <div className="form-grid">
          <Field label="Bezeichnung" required error={errors.label}>
            {(p) => (
              <input
                {...p}
                type="text"
                value={values.label}
                onChange={(e) => set("label", e.target.value)}
                autoComplete="off"
              />
            )}
          </Field>

          <fieldset className="field radio-group">
            <legend>Ziel</legend>
            <label className="inline">
              <input
                type="radio"
                name="target"
                checked={!isLive}
                onChange={() => chooseTarget("local")}
              />
              Lokal
            </label>
            <label className="inline">
              <input
                type="radio"
                name="target"
                checked={isLive}
                onChange={() => chooseTarget("live")}
              />
              Live
            </label>
          </fieldset>
        </div>

        {isLive ? (
          <div className="notice notice-warning" role="note">
            <p>{LIVE_BOX_HINT}</p>
            <label className="inline">
              <input
                type="checkbox"
                checked={liveConfirmed}
                onChange={(e) => setLiveConfirmed(e.target.checked)}
              />
              Ich nutze die Kennung einer Simulationsanlage.
            </label>
          </div>
        ) : null}

        <Field
          label="Adresse der Zentrale"
          required
          error={errors.baseUrl}
          hint="Die Kennung wird beim Verbinden angehängt. Adressen außerhalb des lokalen Netzes brauchen das Ziel Live."
        >
          {(p) => (
            <input
              {...p}
              type="text"
              className="mono"
              value={values.baseUrl}
              onChange={(e) => {
                setUrlEdited(true);
                set("baseUrl", e.target.value);
              }}
              autoComplete="off"
              spellCheck={false}
            />
          )}
        </Field>

        <div className="form-grid">
          <Field label="Kennung" required error={errors.identity}>
            {(p) => (
              <input
                {...p}
                type="text"
                className="mono"
                value={values.identity}
                onChange={(e) => set("identity", e.target.value)}
                autoComplete="off"
                spellCheck={false}
              />
            )}
          </Field>
          <Field label="Passwort" required error={errors.password}>
            {(p) => (
              <input
                {...p}
                type="password"
                value={values.password}
                onChange={(e) => set("password", e.target.value)}
                autoComplete="new-password"
              />
            )}
          </Field>
          <Field label="Hersteller" required error={errors.vendor}>
            {(p) => (
              <input
                {...p}
                type="text"
                value={values.vendor}
                onChange={(e) => set("vendor", e.target.value)}
              />
            )}
          </Field>
          <Field label="Modell" required error={errors.model}>
            {(p) => (
              <input
                {...p}
                type="text"
                value={values.model}
                onChange={(e) => set("model", e.target.value)}
              />
            )}
          </Field>
          <Field label="Phasen" required>
            {(p) => (
              <select
                {...p}
                value={values.phases}
                onChange={(e) => set("phases", e.target.value === "1" ? 1 : 3)}
              >
                <option value="1">1</option>
                <option value="3">3</option>
              </select>
            )}
          </Field>
          <Field label="Max. Leistung (kW)" required error={errors.maxPowerKw}>
            {(p) => (
              <input
                {...p}
                type="text"
                inputMode="decimal"
                value={values.maxPowerKw}
                onChange={(e) => set("maxPowerKw", e.target.value)}
              />
            )}
          </Field>
          <Field label="Uhrabweichung (s)" required error={errors.clockOffsetS}>
            {(p) => (
              <input
                {...p}
                type="text"
                inputMode="numeric"
                value={values.clockOffsetS}
                onChange={(e) => set("clockOffsetS", e.target.value)}
              />
            )}
          </Field>
        </div>

        <fieldset className="field">
          <legend>Verhalten der Wallbox</legend>
          <label className="inline">
            <input
              type="checkbox"
              checked={values.supportsSoc}
              onChange={(e) => set("supportsSoc", e.target.checked)}
            />
            Ladestand (SoC) melden
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={values.unitW}
              onChange={(e) => set("unitW", e.target.checked)}
            />
            Leistungseinheit W akzeptieren
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={values.unitA}
              onChange={(e) => set("unitA", e.target.checked)}
            />
            Leistungseinheit A akzeptieren
          </label>
          {errors.unitW ? <p className="field-error">{errors.unitW}</p> : null}
          <label className="inline">
            <input
              type="checkbox"
              checked={values.rejectProfiles}
              onChange={(e) => set("rejectProfiles", e.target.checked)}
            />
            Ladeprofile ablehnen
          </label>
        </fieldset>

        {submitError ? (
          <p className="notice notice-critical" role="alert">
            {submitError}
          </p>
        ) : null}

        <div className="dialog-actions">
          <button type="button" className="btn" onClick={close}>
            Abbrechen
          </button>
          <button
            type="submit"
            className="btn btn-primary"
            disabled={submitting || (isLive && !liveConfirmed)}
          >
            {submitting ? "Wird angelegt …" : "Anlegen"}
          </button>
        </div>
      </form>
    </dialog>
  );
}
