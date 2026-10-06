# First week

1. Read `README.md`, `ARCHITECTURE.md`, `docs/CONTRACT.md`, `docs/CENTRAL_SYSTEM_BEHAVIOUR.md`.
2. Run the app locally (`README.md`, "Run locally") and add a wallbox against a local backend.
3. Run scenario S1 and S2, open the "Protokoll" tab and follow the OCPP frames: BootNotification, the
   post-boot ChangeConfiguration calls, StatusNotification, StartTransaction, MeterValues.
4. Read `crates/sim-core/src/charge_point/profiles/` and its tests: this is the logic that decides how much
   a wallbox may charge. The tests show every rule with a counter-check.
5. Read `crates/sim-core/src/policy.rs` and `gate.rs`: the rules that protect the production server.
6. Change something small (a German text, a scenario detail), run the full check list from
   `CONTRIBUTING.md`, commit with an explicit file list.

## Terms

| Term | Meaning |
|---|---|
| Zentrale / central system | The OCPP server in the Ampiera backend (worker, port 9000, `wss://api.ampiera.de/ocpp/<Kennung>`) |
| Kennung / identity | OCPP charge point id issued by the backend (`AP` + 10 characters) |
| Ladeprofil / charging profile | `SetChargingProfile` from the central system; limits power until `validTo` |
| Testgrenze | Commissioning tool in the intranet that sends a profile of at most 15 minutes |
| Simulationsanlage | Installation with `istSimulation = true`; the only kind a simulated wallbox may belong to |
| Produktivserver / Live | `api.ampiera.de`; marked in amber everywhere in the UI |
