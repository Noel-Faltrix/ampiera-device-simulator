import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "../api/client";
import { AddBoxDialog } from "./AddBoxDialog";

vi.mock("../api/client");

async function fillBasics(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("Bezeichnung"), "Box 1");
  await user.type(screen.getByLabelText("Kennung"), "AP7K2M9QX4RT");
  await user.type(screen.getByLabelText("Passwort"), "geheim");
}

describe("AddBoxDialog", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.addChargePoint).mockResolvedValue("new-id");
  });

  it("prefills the contract defaults", () => {
    render(<AddBoxDialog onClose={() => {}} onAdded={() => {}} />);
    expect(screen.getByLabelText("Server-Adresse")).toHaveValue("ws://localhost:9000/ocpp");
    expect(screen.getByLabelText("Hersteller")).toHaveValue("Ampiera Sim");
    expect(screen.getByLabelText("Modell")).toHaveValue("Device Simulator");
    expect(screen.getByLabelText("Max. Leistung (kW)")).toHaveValue("11");
    expect(screen.getByLabelText("Passwort")).toHaveAttribute("type", "password");
    expect(screen.queryByText(/Produktivserver/)).not.toBeInTheDocument();
  });

  it("shows the live warning and blocks submitting until it is confirmed", async () => {
    const user = userEvent.setup();
    render(<AddBoxDialog onClose={() => {}} onAdded={() => {}} />);
    await fillBasics(user);

    await user.click(screen.getByLabelText("Live"));
    expect(
      screen.getByText(/Diese Box verbindet sich mit dem Produktivserver api\.ampiera\.de/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Server-Adresse")).toHaveValue("wss://api.ampiera.de/ocpp");

    const submit = screen.getByRole("button", { name: "Anlegen" });
    expect(submit).toBeDisabled();
    await user.click(submit);
    expect(api.addChargePoint).not.toHaveBeenCalled();

    await user.click(screen.getByLabelText(/Ich verstehe das/));
    expect(submit).toBeEnabled();
    await user.click(submit);

    await waitFor(() => expect(api.addChargePoint).toHaveBeenCalledTimes(1));
    const call = vi.mocked(api.addChargePoint).mock.calls[0]![0];
    expect(call.liveConfirmed).toBe(true);
    expect(call.config.targetKind).toBe("live");
    expect(call.password).toBe("geheim");
  });

  it("submits a local box without any confirmation", async () => {
    const user = userEvent.setup();
    const onAdded = vi.fn();
    const onClose = vi.fn();
    render(<AddBoxDialog onClose={onClose} onAdded={onAdded} />);
    await fillBasics(user);
    await user.click(screen.getByRole("button", { name: "Anlegen" }));

    await waitFor(() => expect(onAdded).toHaveBeenCalledWith("new-id"));
    expect(onClose).toHaveBeenCalled();
    expect(vi.mocked(api.addChargePoint).mock.calls[0]![0].liveConfirmed).toBe(false);
  });

  it("resets the confirmation when switching back to local", async () => {
    const user = userEvent.setup();
    render(<AddBoxDialog onClose={() => {}} onAdded={() => {}} />);
    await user.click(screen.getByLabelText("Live"));
    await user.click(screen.getByLabelText(/Ich verstehe das/));
    await user.click(screen.getByLabelText("Lokal"));
    await user.click(screen.getByLabelText("Live"));
    expect(screen.getByLabelText(/Ich verstehe das/)).not.toBeChecked();
  });

  it("rejects an unencrypted address outside the private network and an invalid kennung", async () => {
    const user = userEvent.setup();
    render(<AddBoxDialog onClose={() => {}} onAdded={() => {}} />);
    await fillBasics(user);
    const url = screen.getByLabelText("Server-Adresse");
    await user.clear(url);
    await user.type(url, "ws://api.ampiera.de/ocpp");
    await user.clear(screen.getByLabelText("Kennung"));
    await user.type(screen.getByLabelText("Kennung"), "mit leerzeichen");
    await user.click(screen.getByRole("button", { name: "Anlegen" }));

    expect(screen.getByText(/nur für localhost und private Adressen/)).toBeInTheDocument();
    expect(screen.getByText(/1 bis 48 Zeichen/)).toBeInTheDocument();
    expect(api.addChargePoint).not.toHaveBeenCalled();
  });

  it("shows the error of the core inside the dialog", async () => {
    vi.mocked(api.addChargePoint).mockRejectedValue("Es sind bereits 3 Live-Boxen verbunden.");
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<AddBoxDialog onClose={onClose} onAdded={() => {}} />);
    await fillBasics(user);
    await user.click(screen.getByRole("button", { name: "Anlegen" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Es sind bereits 3 Live-Boxen verbunden.",
    );
    expect(onClose).not.toHaveBeenCalled();
  });
});
