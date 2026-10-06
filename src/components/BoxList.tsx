import { useSim } from "../state/store";
import { ConnectionIndicator, TargetBadge } from "./badges";

interface BoxListProps {
  onAdd: () => void;
}

export function BoxList({ onAdd }: BoxListProps) {
  const { state, dispatch } = useSim();
  const boxes = state.order.map((id) => state.boxes[id]).filter((b) => b !== undefined);

  return (
    <aside className="sidebar" aria-label="Wallboxen">
      <div className="sidebar-head">
        <h2>Wallboxen</h2>
        <button type="button" className="btn btn-primary" onClick={onAdd}>
          Wallbox hinzufügen
        </button>
      </div>
      {state.restoreProblems.length > 0 ? (
        <ul className="problems" aria-label="Hinweise zur Wiederherstellung">
          {state.restoreProblems.map((message, index) => (
            <li key={`${index}-${message}`} className="notice notice-warning problem">
              <span>Nicht wiederhergestellt: {message}</span>
              <button
                type="button"
                className="btn"
                onClick={() => dispatch({ type: "dismissProblem", index })}
              >
                Schließen
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {state.loadError ? (
        <p className="notice notice-critical" role="alert">
          Die Wallboxen konnten nicht geladen werden: {state.loadError}
        </p>
      ) : null}
      {boxes.length === 0 ? (
        state.loadError ? null : state.ready ? (
          <p className="empty">
            Noch keine Wallbox angelegt. Lege eine an, um dich mit der Zentrale zu verbinden.
          </p>
        ) : (
          <p className="empty">Wallboxen werden geladen.</p>
        )
      ) : (
        <ul className="box-list">
          {boxes.map((box) => (
            <li
              key={box.id}
              className={`box-row${box.config.targetKind === "live" ? " box-row-live" : ""}${
                box.id === state.selectedId ? " is-selected" : ""
              }`}
            >
              <button
                type="button"
                aria-current={box.id === state.selectedId ? "true" : undefined}
                onClick={() => dispatch({ type: "select", id: box.id })}
              >
                <span className="box-row-top">
                  <span className="box-label">{box.config.label}</span>
                  <TargetBadge kind={box.config.targetKind} />
                </span>
                <span className="mono box-identity">{box.config.identity}</span>
                <ConnectionIndicator box={box} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
