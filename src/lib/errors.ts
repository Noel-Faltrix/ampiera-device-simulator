/** Tauri rejects with the German string of the core; anything else gets a generic text. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string" && error.length > 0) return error;
  if (error instanceof Error && error.message.length > 0) return error.message;
  return "Unbekannter Fehler.";
}
