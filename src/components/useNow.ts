import { useSyncExternalStore } from "react";

function subscribeToTicks(onTick: () => void): () => void {
  const timer = window.setInterval(onTick, 1000);
  return () => window.clearInterval(timer);
}

const noSubscription = () => () => {};

// Quantised to whole seconds so repeated reads between ticks return the same value.
const readNow = () => Math.floor(Date.now() / 1000) * 1000;

/** Current time in ms; re-renders every second while `active`. */
export function useNow(active: boolean): number {
  return useSyncExternalStore(active ? subscribeToTicks : noSubscription, readNow);
}
