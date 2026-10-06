import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  type Dispatch,
  type ReactNode,
} from "react";
import * as api from "../api/client";
import type {
  AppViewSnapshot,
  ChargePointSnapshot,
  FrameLogEntry,
  ScenarioId,
  ScenarioInfo,
  ScenarioReport,
} from "../api/types";
import { FRAME_CAP } from "../lib/constants";
import { errorMessage } from "../lib/errors";

export type AppAuth = "loggedOut" | "codeRequired" | "loggedIn";

export interface SimState {
  ready: boolean;
  loadError: string | null;
  order: string[];
  boxes: Record<string, ChargePointSnapshot>;
  frames: Record<string, FrameLogEntry[]>;
  selectedId: string | null;
  scenarios: ScenarioInfo[];
  scenariosError: string | null;
  reports: Record<string, ScenarioReport>;
  /** Markdown of each report, fetched when the report arrives so copying needs no await. */
  reportMarkdown: Record<string, string>;
  restoreProblems: string[];
  running: Record<string, { scenarioId: ScenarioId; startedAt: number }>;
  app: { auth: AppAuth; snapshot: AppViewSnapshot | null; error: string | null; live: boolean };
}

export type SimAction =
  | { type: "loaded"; boxes: ChargePointSnapshot[] }
  | { type: "restoreProblem"; message: string }
  | { type: "dismissProblem"; index: number }
  | { type: "reportMarkdown"; key: string; markdown: string }
  | { type: "loadFailed"; message: string }
  | { type: "updated"; snapshot: ChargePointSnapshot }
  | { type: "removed"; id: string }
  | { type: "frame"; entry: FrameLogEntry }
  | { type: "select"; id: string }
  | { type: "scenarios"; scenarios: ScenarioInfo[] }
  | { type: "scenariosFailed"; message: string }
  | { type: "report"; report: ScenarioReport }
  | { type: "runStarted"; id: string; scenarioId: ScenarioId; at: number }
  | { type: "runEnded"; id: string }
  | { type: "appAuth"; auth: AppAuth; live?: boolean }
  | { type: "appSnapshot"; snapshot: AppViewSnapshot }
  | { type: "appError"; message: string };

export const initialState: SimState = {
  ready: false,
  loadError: null,
  order: [],
  boxes: {},
  frames: {},
  selectedId: null,
  scenarios: [],
  scenariosError: null,
  reports: {},
  reportMarkdown: {},
  restoreProblems: [],
  running: {},
  app: { auth: "loggedOut", snapshot: null, error: null, live: false },
};

export const reportKey = (boxId: string, scenarioId: ScenarioId) => `${boxId}:${scenarioId}`;

function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

function upsert(state: SimState, snapshot: ChargePointSnapshot): SimState {
  const known = snapshot.id in state.boxes;
  return {
    ...state,
    order: known ? state.order : [...state.order, snapshot.id],
    boxes: { ...state.boxes, [snapshot.id]: snapshot },
    selectedId: state.selectedId ?? snapshot.id,
  };
}

export function reducer(state: SimState, action: SimAction): SimState {
  switch (action.type) {
    case "loaded": {
      // An event may have delivered a newer snapshot while the list call was in flight.
      const fresh = action.boxes.filter((box) => {
        const known = state.boxes[box.id];
        return !known || Date.parse(known.updatedAt) <= Date.parse(box.updatedAt);
      });
      const next = fresh.reduce(upsert, state);
      return { ...next, ready: true, loadError: null };
    }
    case "restoreProblem":
      return { ...state, restoreProblems: [...state.restoreProblems, action.message] };
    case "dismissProblem":
      return {
        ...state,
        restoreProblems: state.restoreProblems.filter((_, i) => i !== action.index),
      };
    case "reportMarkdown":
      return {
        ...state,
        reportMarkdown: { ...state.reportMarkdown, [action.key]: action.markdown },
      };
    case "loadFailed":
      return { ...state, ready: true, loadError: action.message };
    case "updated":
      return upsert(state, action.snapshot);
    case "removed": {
      const order = state.order.filter((id) => id !== action.id);
      const boxes = omit(state.boxes, action.id);
      const frames = omit(state.frames, action.id);
      const prefix = `${action.id}:`;
      const reports = Object.fromEntries(
        Object.entries(state.reports).filter(([key]) => !key.startsWith(prefix)),
      );
      const running = omit(state.running, action.id);
      return {
        ...state,
        order,
        boxes,
        frames,
        reports,
        reportMarkdown: Object.fromEntries(
          Object.entries(state.reportMarkdown).filter(([key]) => !key.startsWith(prefix)),
        ),
        running,
        selectedId: state.selectedId === action.id ? (order[0] ?? null) : state.selectedId,
      };
    }
    case "frame": {
      const id = action.entry.chargePointId;
      const list = state.frames[id] ?? [];
      const next =
        list.length >= FRAME_CAP
          ? [...list.slice(list.length - FRAME_CAP + 1), action.entry]
          : [...list, action.entry];
      return { ...state, frames: { ...state.frames, [id]: next } };
    }
    case "select":
      return { ...state, selectedId: action.id };
    case "scenarios":
      return { ...state, scenarios: action.scenarios, scenariosError: null };
    case "scenariosFailed":
      return { ...state, scenariosError: action.message };
    case "report":
      return {
        ...state,
        reports: {
          ...state.reports,
          [reportKey(action.report.chargePointId, action.report.scenarioId)]: action.report,
        },
      };
    case "runStarted":
      return {
        ...state,
        running: {
          ...state.running,
          [action.id]: { scenarioId: action.scenarioId, startedAt: action.at },
        },
      };
    case "runEnded": {
      const running = omit(state.running, action.id);
      return { ...state, running };
    }
    case "appAuth":
      return {
        ...state,
        app: {
          auth: action.auth,
          snapshot: action.auth === "loggedOut" ? null : state.app.snapshot,
          error: null,
          live: action.auth === "loggedOut" ? false : (action.live ?? state.app.live),
        },
      };
    case "appSnapshot":
      return { ...state, app: { ...state.app, snapshot: action.snapshot, error: null } };
    case "appError":
      return { ...state, app: { ...state.app, error: action.message } };
  }
}

interface SimContextValue {
  state: SimState;
  dispatch: Dispatch<SimAction>;
}

const SimContext = createContext<SimContextValue | null>(null);

export function SimProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);

  useEffect(() => {
    let cancelled = false;
    let unsubscribe: (() => void) | null = null;

    // Subscribe before listing so no update between the two calls is lost.
    void (async () => {
      try {
        const off = await api.subscribe({
          onChargePointUpdated: (snapshot) => dispatch({ type: "updated", snapshot }),
          onFrameLogged: (entry) => dispatch({ type: "frame", entry }),
          onChargePointRemoved: (id) => dispatch({ type: "removed", id }),
          onRestoreProblem: (message) => dispatch({ type: "restoreProblem", message }),
        });
        if (cancelled) off();
        else unsubscribe = off;
        const boxes = await api.listChargePoints();
        if (!cancelled) dispatch({ type: "loaded", boxes });
      } catch (error) {
        if (!cancelled) dispatch({ type: "loadFailed", message: errorMessage(error) });
      }
    })();

    void api
      .listScenarios()
      .then((scenarios) => {
        if (!cancelled) dispatch({ type: "scenarios", scenarios });
      })
      .catch((error: unknown) => {
        if (!cancelled) dispatch({ type: "scenariosFailed", message: errorMessage(error) });
      });

    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, []);

  const value = useMemo(() => ({ state, dispatch }), [state]);
  return <SimContext.Provider value={value}>{children}</SimContext.Provider>;
}

export function useSim(): SimContextValue {
  const ctx = useContext(SimContext);
  if (!ctx) throw new Error("useSim must be used inside SimProvider");
  return ctx;
}
