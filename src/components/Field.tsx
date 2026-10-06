import { useId, type ReactNode } from "react";

interface ControlProps {
  id: string;
  "aria-invalid"?: true;
  "aria-describedby"?: string;
  "aria-required"?: true;
}

interface FieldProps {
  label: string;
  error?: string | undefined;
  hint?: string | undefined;
  required?: boolean;
  children: (props: ControlProps) => ReactNode;
}

export function Field({ label, error, hint, required, children }: FieldProps) {
  const id = useId();
  const messageId = `${id}-msg`;
  const hasMessage = Boolean(error ?? hint);
  const props: ControlProps = { id };
  if (error) props["aria-invalid"] = true;
  if (required) props["aria-required"] = true;
  if (hasMessage) props["aria-describedby"] = messageId;
  return (
    <div className="field">
      <label htmlFor={id} className={required ? "required" : undefined}>
        {label}
      </label>
      {children(props)}
      {error ? (
        <p id={messageId} className="field-error" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p id={messageId} className="field-hint">
          {hint}
        </p>
      ) : null}
    </div>
  );
}
