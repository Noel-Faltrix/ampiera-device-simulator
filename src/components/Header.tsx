import { useState } from "react";
import { applyTheme, readStoredTheme, type ThemeChoice } from "../lib/theme";

const CHOICES: { id: ThemeChoice; label: string }[] = [
  { id: "light", label: "Hell" },
  { id: "dark", label: "Dunkel" },
  { id: "system", label: "System" },
];

function HexagonMark() {
  return (
    <svg
      className="mark"
      viewBox="0 0 24 24"
      width="24"
      height="24"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
    </svg>
  );
}

export function Header() {
  const [theme, setTheme] = useState<ThemeChoice>(readStoredTheme);

  function choose(choice: ThemeChoice) {
    setTheme(choice);
    applyTheme(choice);
  }

  return (
    <header className="app-header">
      <div className="brand">
        <HexagonMark />
        <h1>Ampiera Device Simulator</h1>
      </div>
      <div role="group" aria-label="Farbschema" className="theme-toggle">
        {CHOICES.map((c) => (
          <button
            key={c.id}
            type="button"
            aria-pressed={theme === c.id}
            onClick={() => choose(c.id)}
          >
            {c.label}
          </button>
        ))}
      </div>
    </header>
  );
}
