import { useState } from "react";
import { AddBoxDialog } from "./components/AddBoxDialog";
import { BoxDetail } from "./components/BoxDetail";
import { BoxList } from "./components/BoxList";
import { Header } from "./components/Header";
import { SimProvider, useSim } from "./state/store";

function Shell() {
  const { state, dispatch } = useSim();
  const [adding, setAdding] = useState(false);
  const selected = state.selectedId ? state.boxes[state.selectedId] : undefined;

  return (
    <div className="app">
      <Header />
      <div className="app-body">
        <BoxList onAdd={() => setAdding(true)} />
        <main className="main">
          {selected ? (
            <BoxDetail key={selected.id} box={selected} />
          ) : state.ready ? (
            <p className="empty">
              Keine Wallbox ausgewählt. Lege eine an oder wähle links eine aus, um Status, Szenarien
              und Protokoll zu sehen.
            </p>
          ) : null}
        </main>
      </div>
      {adding ? (
        <AddBoxDialog
          onClose={() => setAdding(false)}
          onAdded={(id) => dispatch({ type: "select", id })}
        />
      ) : null}
    </div>
  );
}

export function App() {
  return (
    <SimProvider>
      <Shell />
    </SimProvider>
  );
}
