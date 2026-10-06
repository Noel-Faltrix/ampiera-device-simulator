import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api/client";
import type { ScenarioReport } from "./api/types";
import { App } from "./App";
import { makeBox, makeScenario } from "./test/fixtures";

vi.mock("./api/client");

const scenarios = [
  makeScenario({ id: "S1", title: "Anmelden, Status, Heartbeat" }),
  makeScenario({ id: "S10", title: "Falsches Passwort", liveAllowed: false, timeoutS: 60 }),
  makeScenario({
    id: "S3",
    title: "Testgrenze aus dem Intranet",
    needsHuman: true,
    timeoutS: 1200,
  }),
];

function setup(boxes = [makeBox()]) {
  vi.mocked(api.subscribe).mockResolvedValue(() => {});
  vi.mocked(api.listChargePoints).mockResolvedValue(boxes);
  vi.mocked(api.listScenarios).mockResolvedValue(scenarios);
  return userEvent.setup();
}

async function openTab(user: ReturnType<typeof userEvent.setup>, name: string) {
  await screen.findByRole("tab", { name });
  await user.click(screen.getByRole("tab", { name }));
}

describe("App", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("explains the empty state", async () => {
    setup([]);
    render(<App />);
    expect(
      await screen.findByText(
        "Noch keine Wallbox angelegt. Lege eine an, um dich mit der Zentrale zu verbinden.",
      ),
    ).toBeInTheDocument();
  });

  it("marks a live box in the list", async () => {
    setup([makeBox({}, true), makeBox()]);
    render(<App />);
    const list = await screen.findByRole("list");
    const rows = within(list).getAllByRole("listitem");
    expect(within(rows[0]!).getByText("Live")).toBeInTheDocument();
    expect(rows[0]).toHaveClass("box-row-live");
    expect(within(rows[1]!).getByText("Lokal")).toBeInTheDocument();
    expect(rows[1]).not.toHaveClass("box-row-live");
  });

  it("shows power, meter and the limit countdown on the overview", async () => {
    const validTo = new Date(Date.now() + 4 * 60_000 + 32_000).toISOString();
    setup([
      makeBox({
        activeLimit: {
          limitW: 7360,
          rawLimit: 10.7,
          rateUnit: "A",
          profileId: 4711,
          purpose: "TxDefaultProfile",
          validTo,
        },
        powerW: 7400,
      }),
    ]);
    render(<App />);
    expect(await screen.findByText("7,4 kW")).toBeInTheDocument();
    expect(screen.getByText("12,35 kWh")).toBeInTheDocument();
    expect(screen.getByText(/empfangen: 10,7 A/)).toBeInTheDocument();
    expect(screen.getByText(/endet in [34]:\d\d min/)).toBeInTheDocument();
  });

  it("shows a dash for unknown power", async () => {
    setup([makeBox({ powerW: null, transactionId: null })]);
    render(<App />);
    const row = (await screen.findByText("Ladeleistung")).closest("tr")!;
    expect(within(row).getByText("—")).toBeInTheDocument();
  });

  it("shows command errors next to the actions", async () => {
    vi.mocked(api.reboot).mockRejectedValue("Die Box ist nicht verbunden.");
    const user = setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Wallbox neu starten" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Die Box ist nicht verbunden.");
  });

  it("asks for confirmation before removing a box", async () => {
    vi.mocked(api.removeChargePoint).mockResolvedValue();
    const user = setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Entfernen" }));
    expect(api.removeChargePoint).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Entfernen" }));
    expect(api.removeChargePoint).toHaveBeenCalledWith("box-local");
  });

  it("warns on a live wallbox with a banner and not on a local one", async () => {
    setup([makeBox({}, true)]);
    const { unmount } = render(<App />);
    expect(
      await screen.findByText(
        "Live: Diese Wallbox ist mit dem Produktivserver verbunden. Aktionen wirken auf den echten Betrieb.",
      ),
    ).toBeInTheDocument();
    unmount();
    setup([makeBox()]);
    render(<App />);
    await screen.findByText("Stand:", { exact: false });
    expect(screen.queryByText(/Aktionen wirken auf den echten Betrieb/)).not.toBeInTheDocument();
  });

  it("shows the time of the last state change and flags stale values", async () => {
    setup([makeBox({ connection: { state: "disconnected" } })]);
    render(<App />);
    const stand = await screen.findByText(/^Stand: \d\d\.\d\d\.\d{4}, \d\d:\d\d:\d\d/);
    expect(stand).toHaveTextContent("Werte vom letzten Kontakt");
  });

  it("shows restore problems and lets the user dismiss them", async () => {
    let problem: (m: string) => void = () => {};
    vi.mocked(api.subscribe).mockImplementation((handlers) => {
      problem = handlers.onRestoreProblem;
      return Promise.resolve(() => {});
    });
    vi.mocked(api.listChargePoints).mockImplementation(() => {
      problem("Das Passwort fehlt im Schlüsselbund.");
      return Promise.resolve([]);
    });
    vi.mocked(api.listScenarios).mockResolvedValue(scenarios);
    const user = userEvent.setup();
    render(<App />);
    expect(
      await screen.findByText("Nicht wiederhergestellt: Das Passwort fehlt im Schlüsselbund."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Schließen" }));
    expect(screen.queryByText(/Nicht wiederhergestellt/)).not.toBeInTheDocument();
  });

  it("renders the vehicle charge level without inline styles", async () => {
    setup([
      makeBox({
        vehicle: {
          config: { capacityKwh: 60, socPct: 30, maxPowerW: 11000, phases: 3 },
          socPct: 42,
          plugged: true,
        },
      }),
    ]);
    const { container } = render(<App />);
    expect(
      await screen.findByRole("progressbar", { name: "Ladestand des Fahrzeugs" }),
    ).toHaveAttribute("value", "42");
    expect(container.querySelector("[style]")).toBeNull();
  });

  describe("scenarios", () => {
    it("explains why start is disabled while another scenario runs", async () => {
      vi.mocked(api.runScenario).mockReturnValue(new Promise(() => {}));
      const user = setup();
      render(<App />);
      await openTab(user, "Szenarien");
      const first = (await screen.findByText("Anmelden, Status, Heartbeat")).closest("li")!;
      await user.click(within(first).getByRole("button", { name: "Starten" }));
      const other = screen.getByText("Falsches Passwort").closest("li")!;
      const button = within(other).getByRole("button", { name: "Starten" });
      expect(button).toBeDisabled();
      expect(button).toHaveAccessibleDescription("Auf dieser Wallbox läuft bereits ein Szenario.");
      expect(within(first).getByText("Szenario gestartet")).toBeInTheDocument();
    });

    it("disables local-only scenarios on a live box and says why", async () => {
      const user = setup([makeBox({}, true)]);
      render(<App />);
      await openTab(user, "Szenarien");
      const item = (await screen.findByText("Falsches Passwort")).closest("li")!;
      expect(within(item).getByRole("button", { name: "Auf Live starten" })).toBeDisabled();
      expect(within(item).getByText("nur lokal ausführbar")).toBeInTheDocument();
      expect(within(item).getByText(/läuft nur auf lokalen Wallboxen/)).toBeInTheDocument();
      const allowed = screen.getByText("Anmelden, Status, Heartbeat").closest("li")!;
      expect(within(allowed).getByRole("button", { name: "Auf Live starten" })).toBeEnabled();
    });

    it("allows the same scenario on a local box", async () => {
      const user = setup();
      render(<App />);
      await openTab(user, "Szenarien");
      const item = (await screen.findByText("Falsches Passwort")).closest("li")!;
      expect(within(item).getByRole("button", { name: "Starten" })).toBeEnabled();
    });

    it("marks scenarios that need a human", async () => {
      const user = setup();
      render(<App />);
      await openTab(user, "Szenarien");
      const item = (await screen.findByText("Testgrenze aus dem Intranet")).closest("li")!;
      expect(within(item).getByText("Handlung nötig")).toBeInTheDocument();
      expect(within(item).getByText("Zeitlimit 20 min")).toBeInTheDocument();
    });

    it("runs a scenario, shows the report and copies it as markdown", async () => {
      const report: ScenarioReport = {
        scenarioId: "S1",
        chargePointId: "box-local",
        targetKind: "local",
        startedAt: "2026-10-06T10:00:00Z",
        finishedAt: "2026-10-06T10:00:30Z",
        outcome: "failed",
        checks: [
          {
            name: "Boot angenommen",
            outcome: "passed",
            detail: "Die Zentrale hat Accepted gesendet.",
          },
          { name: "Heartbeat", outcome: "failed", detail: "Keine Antwort in 10 s." },
          { name: "Folgeaufrufe", outcome: "skipped", detail: "Übersprungen." },
        ],
      };
      let finish: (r: ScenarioReport) => void = () => {};
      vi.mocked(api.runScenario).mockReturnValue(new Promise((resolve) => (finish = resolve)));
      vi.mocked(api.exportReport).mockResolvedValue("# Bericht");
      const user = setup();
      const writeText = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
      render(<App />);
      await openTab(user, "Szenarien");

      const item = (await screen.findByText("Anmelden, Status, Heartbeat")).closest("li")!;
      await user.click(within(item).getByRole("button", { name: "Starten" }));
      expect(api.runScenario).toHaveBeenCalledWith("box-local", "S1");
      expect(within(item).getByRole("button", { name: "Abbrechen" })).toBeInTheDocument();

      finish(report);
      expect(await within(item).findByText("Ergebnis: nicht bestanden")).toBeInTheDocument();
      expect(within(item).getByText("bestanden")).toBeInTheDocument();
      expect(within(item).getByText("übersprungen")).toBeInTheDocument();
      expect(within(item).getByText("Keine Antwort in 10 s.")).toBeInTheDocument();

      const copyButton = within(item).getByRole("button", { name: "Bericht kopieren" });
      await waitFor(() => expect(copyButton).toBeEnabled());
      await user.click(copyButton);
      await waitFor(() => expect(writeText).toHaveBeenCalledWith("# Bericht"));

      vi.mocked(api.saveReport).mockResolvedValue("/home/u/Downloads/bericht.md");
      await user.click(within(item).getByRole("button", { name: "Bericht speichern" }));
      expect(
        await within(item).findByText("Gespeichert unter /home/u/Downloads/bericht.md"),
      ).toBeInTheDocument();
    });
  });

  describe("protocol", () => {
    it("saves the log through the core and shows the path", async () => {
      vi.mocked(api.saveLog).mockResolvedValue("/home/u/Downloads/protokoll.json");
      const user = setup();
      render(<App />);
      await openTab(user, "Protokoll");
      await user.click(await screen.findByRole("button", { name: "Exportieren" }));
      expect(api.saveLog).toHaveBeenCalledWith("box-local");
      expect(
        await screen.findByText("Gespeichert unter /home/u/Downloads/protokoll.json"),
      ).toBeInTheDocument();
    });

    it("filters frames by action name", async () => {
      let emit: (e: {
        chargePointId: string;
        at: string;
        direction: "out" | "in";
        raw: string;
      }) => void = () => {};
      vi.mocked(api.subscribe).mockImplementation((handlers) => {
        emit = handlers.onFrameLogged;
        return Promise.resolve(() => {});
      });
      vi.mocked(api.listChargePoints).mockResolvedValue([makeBox()]);
      vi.mocked(api.listScenarios).mockResolvedValue(scenarios);
      const user = userEvent.setup();
      render(<App />);
      await openTab(user, "Protokoll");
      expect(await screen.findByText(/Noch kein Datenverkehr/)).toBeInTheDocument();

      const at = "2026-10-06T10:00:00.000Z";
      emit({ chargePointId: "box-local", at, direction: "out", raw: '[2,"1","Heartbeat",{}]' });
      emit({ chargePointId: "box-local", at, direction: "out", raw: '[2,"2","MeterValues",{}]' });
      emit({ chargePointId: "other", at, direction: "out", raw: '[2,"3","Authorize",{}]' });

      expect(await screen.findByText("2 von 2 Einträgen")).toBeInTheDocument();
      await user.type(screen.getByLabelText("Filter nach Aktion"), "meter");
      expect(screen.getByText("1 von 2 Einträgen")).toBeInTheDocument();
      expect(screen.queryByText("Heartbeat")).not.toBeInTheDocument();
      expect(screen.getByText("MeterValues")).toBeInTheDocument();
      expect(screen.getAllByText("an die Zentrale")).toHaveLength(1);
    });
  });

  describe("app view", () => {
    it("asks for the code after a login that needs one", async () => {
      vi.mocked(api.appLogin).mockResolvedValue({ result: "device_code_required" });
      const user = setup();
      render(<App />);
      await openTab(user, "App-Sicht");
      await screen.findByRole("heading", { name: "Als Testkunde anmelden" });
      const loginForm = screen
        .getByRole("heading", { name: "Als Testkunde anmelden" })
        .closest("form")!;
      await user.type(within(loginForm).getByLabelText("E-Mail"), "kunde@example.org");
      await user.type(within(loginForm).getByLabelText("Passwort"), "pw");
      await user.click(within(loginForm).getByRole("button", { name: "Anmelden" }));
      expect(
        await screen.findByText(
          "Der Server hat einen Anmeldecode an die E-Mail-Adresse des Testkunden geschickt.",
        ),
      ).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Bestätigen" })).toBeDisabled();
    });

    it("shows a length hint for the new password of an invitation", async () => {
      const user = setup();
      render(<App />);
      await openTab(user, "App-Sicht");
      await user.click(await screen.findByText("Einladung einlösen", { selector: "summary" }));
      await user.type(screen.getByLabelText("Neues Passwort"), "kurz");
      expect(screen.getByText("4 von mindestens 15 Zeichen")).toBeInTheDocument();
    });
  });
});
