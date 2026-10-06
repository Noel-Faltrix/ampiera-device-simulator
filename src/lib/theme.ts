export type ThemeChoice = "system" | "light" | "dark";

const KEY = "ampiera-sim-theme";

export function readStoredTheme(): ThemeChoice {
  try {
    const value = localStorage.getItem(KEY);
    if (value === "light" || value === "dark") return value;
  } catch {
    // Storage can be unavailable; fall back to the system setting.
  }
  return "system";
}

export function applyTheme(choice: ThemeChoice): void {
  if (choice === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", choice);
  try {
    if (choice === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    // Not persisting is acceptable.
  }
}

export function applyStoredTheme(): void {
  const choice = readStoredTheme();
  if (choice !== "system") document.documentElement.setAttribute("data-theme", choice);
}
