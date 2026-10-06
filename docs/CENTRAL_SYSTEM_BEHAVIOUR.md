# How the Ampiera central system behaves (what the simulator must match)

Read from `ampiera-backend` at commit `b553400` (06.10.2026), `src/services/befehlsweg/ocpp/`. Not verified
against a running server. If the backend changes, update this file and the scenarios.

## Connection

- URL `<base>/<identity>`, path exactly `/ocpp/<identity>`, identity `^[A-Za-z0-9._-]{1,48}$`
  (`ocppRegeln.ts:56-68`). Identities issued by the backend: `AP` + 10 chars.
- HTTP Basic auth, user must equal the identity in the path. Wrong → 401 (no hint whether identity or
  password was wrong). Unknown path → 404. More than 20 failures per IP in 15 min → 429
  (`ocppZentrale.ts`, `FEHLVERSUCHE_MAX = 20`).
- Subprotocol `ocpp1.6`. Offering subprotocols without `ocpp1.6` → 400; offering none is accepted.
- A new connection of the same station closes the old one (close code 1000, reason "ersetzt").
- Server WebSocket ping every 60 s; a client that sent neither pong nor a message since the last ping is
  dropped. Max message size 64 KiB. More than 50 queued incoming messages → disconnect.
- Calls from the central system go out strictly one at a time per station; timeout 30 s (10 s for the
  calls right after boot).

## Responses of the central system

| Call from box | Response |
|---|---|
| BootNotification | requires `chargePointVendor`, `chargePointModel` (≤ 20 chars); always `{status:"Accepted", currentTime, interval:300}` |
| Heartbeat | `{currentTime}` |
| StatusNotification | `{}`; status must be one of the 9 OCPP 1.6 values, else CALLERROR FormationViolation; connectorId > 1 ignored |
| MeterValues | `{}` always |
| Authorize | `{idTagInfo:{status:"Accepted"}}` for any idTag |
| StartTransaction | requires connectorId ≥ 1, meterStart ≥ 0, idTag ≤ 20 chars; `{transactionId:<int ≥ 1>, idTagInfo:{status:"Accepted"}}`; same (connector, meterStart) while open → same id |
| StopTransaction | requires integer transactionId, meterStop ≥ 0; always Accepted, also for unknown id or 0 |
| DataTransfer | `{status:"UnknownVendorId"}` |
| other | CALLERROR NotImplemented |

`transactionId 0` is only returned when the customer is in the recycle bin and the device is not a § 14a
device; nothing is stored, the box keeps charging (`ocppSpeicher.ts:89-91`).

## Calls the central system sends

After an accepted BootNotification, in this order:

1. `ChangeConfiguration {key:"MeterValuesSampledData", value:"Power.Active.Import,Energy.Active.Import.Register,SoC"}`
2. if not `Accepted`: the same without `SoC`
3. `ChangeConfiguration {key:"MeterValueSampleInterval", value:"60"}`
4. `TriggerMessage {requestedMessage:"StatusNotification"}`

On demand (intranet commissioning tools, `POST /api/v1/geraete/:id/ocpp/aktion`, staff + VPN):
`TriggerMessage` (StatusNotification / MeterValues connectorId 1), `RemoteStartTransaction {connectorId:1,
idTag:"AMPIERA"}`, `RemoteStopTransaction {transactionId}`, `Reset {type:"Soft"}`, test limit
(SetChargingProfile, at most 15 min), release (ClearChargingProfile id 4711 and 4712).

### SetChargingProfile

```json
{ "connectorId": 0,
  "csChargingProfiles": {
    "chargingProfileId": 4711, "stackLevel": 1,
    "chargingProfilePurpose": "TxDefaultProfile", "chargingProfileKind": "Absolute",
    "validTo": "<gueltigBis>",
    "chargingSchedule": { "startSchedule": "<now - 60 s>", "chargingRateUnit": "A",
      "chargingSchedulePeriod": [ { "startPeriod": 0, "limit": 10.7, "numberPhases": 3 } ] } } }
```

- Unit `A` (station default): `limit = floor(W / (230 * phases) * 10) / 10`. Unit `W`: `limit = round(W)`,
  no `numberPhases`. "Hold" = limit 0.
- With a running transaction a second profile follows: `connectorId 1`, id 4712, purpose `TxProfile`,
  `transactionId`, same `validTo`.
- Expected answer `{status:"Accepted"}`; anything else is a rejection. ClearChargingProfile: `Unknown`
  also counts as success. No answer in 30 s → retried later (~5 min).
- Profiles already expired are never sent. `validTo` is the end of the box's obligation: after it the box
  must charge as without Ampiera (rules §8 of the Ampiera main rule set).

## MeterValues parsing

- `Power.Active.Import` (unit `kW` ×1000, otherwise W); per-phase values L1/L2/L3 summed if no total.
- `SoC` 0..100. `Energy.Active.Import.Register` read but not stored.
- Timestamp older than 10 min vs. server clock → measurement discarded; more than 10 min in the future or
  missing → server time.
- Stored in `geraete_messwerte` (`quelle = "ocpp"`). Heartbeat/StatusNotification create "rest" points
  (0 W when surely not charging, last power while Charging, null for SuspendedEV/EVSE).

## What the customer app sees of an OCPP wallbox (gap found 06.10.2026)

- OCPP data is written to `geraete_messwerte`, but `GET /dashboard` (`live.wallbox_leistung_w`), the
  geo-position view and `ladebedarf.geladen_kwh` read `telemetrie`, which OCPP never writes. These fields
  stay `null` for an OCPP-only wallbox.
- `Geraet.status` is never set by the OCPP code → the app shows the wallbox as `offline` / "Nicht
  erreichbar".
- Visible today: dashboard `verbindung`/`letzter_kontakt` (from the newest measurement) and
  `GET /geraete-energie` (quarter-hour kWh, computed every 15 min, 3 min after the quarter hour).

Scenario S11 checks exactly this and reports it; fixing it is backend work, not part of the simulator.
