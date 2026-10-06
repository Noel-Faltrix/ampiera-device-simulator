import { useEffect, useRef, useState, type FormEvent } from "react";
import * as api from "../api/client";
import type { TargetKind } from "../api/types";
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
    if (Object.keys(found).length > 0) return;
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
      onClose();
    } catch (error) {
      setSubmitError(errorMessage(error));
      setSubmitting(false);
    }
  }

  return (
    <dialog
      ref={dialogRef}
      className="dialog"
      aria-labelledby="add-box-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <form onSubmit={(e) => void submit(e)} noValidate>
        <h2 id="add-box-title">Wallbox hinzufügen</h2>

        <div className="form-grid">
          <Field label="Bezeichnung" error={errors.label}>
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
            <p>
              Diese Box verbindet sich mit dem Produktivserver api.ampiera.de. Nur Kennungen einer
              Simulationsanlage verwenden. Höchstens 3 Live-Boxen gleichzeitig.
            </p>
            <label className="inline">
              <input
                type="checkbox"
                checked={liveConfirmed}
                onChange={(e) => setLiveConfirmed(e.target.checked)}
              />
              Ich verstehe das und nutze eine Kennung einer Simulationsanlage.
            </label>
          </div>
        ) : null}

        <Field
          label="Server-Adresse"
          error={errors.baseUrl}
          hint="Die Kennung wird beim Verbinden angehängt."
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
          <Field label="Kennung" error={errors.identity}>
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
          <Field label="Passwort" error={errors.password}>
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
          <Field label="Hersteller" error={errors.vendor}>
            {(p) => (
              <input
                {...p}
                type="text"
                value={values.vendor}
                onChange={(e) => set("vendor", e.target.value)}
              />
            )}
          </Field>
          <Field label="Modell" error={errors.model}>
            {(p) => (
              <input
                {...p}
                type="text"
                value={values.model}
                onChange={(e) => set("model", e.target.value)}
              />
            )}
          </Field>
          <Field label="Phasen">
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
          <Field label="Max. Leistung (kW)" error={errors.maxPowerKw}>
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
          <Field label="Uhrabweichung (s)" error={errors.clockOffsetS}>
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
          <legend>Verhalten der Box</legend>
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
            Einheit W akzeptieren
          </label>
          <label className="inline">
            <input
              type="checkbox"
              checked={values.unitA}
              onChange={(e) => set("unitA", e.target.checked)}
            />
            Einheit A akzeptieren
          </label>
          {errors.unitW ? <p className="field-error">{errors.unitW}</p> : null}
          <label className="inline">
            <input
              type="checkbox"
              checked={values.rejectProfiles}
              onChange={(e) => set("rejectProfiles", e.target.checked)}
            />
            Profile ablehnen
          </label>
        </fieldset>

        {submitError ? (
          <p className="notice notice-critical" role="alert">
            {submitError}
          </p>
        ) : null}

        <div className="dialog-actions">
          <button type="button" className="btn" onClick={onClose}>
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
