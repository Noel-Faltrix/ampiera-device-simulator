# Scenarios

Each scenario runs against one simulated charge point and ends with a report of checks
(`passed` / `failed` / `skipped`, German detail text). Times are maxima; a scenario ends early when all
checks are decided. See `CENTRAL_SYSTEM_BEHAVIOUR.md` for the expected server behaviour.

| Id | Title (UI, German) | Live | Human | Timeout | Steps | Checks |
|---|---|---|---|---|---|---|
| S1 | Anmelden, Status, Heartbeat | yes | no | 90 s | connect (if not connected), BootNotification, wait for the post-boot calls | Boot answered `Accepted` with `interval`; the four post-boot calls arrive in order; a Heartbeat is answered with `currentTime` |
| S2 | Auto anstecken und laden | yes | no | 180 s | plug in default vehicle, StatusNotification Preparing, StartTransaction, Charging, two MeterValues | transactionId ≥ 1; MeterValues answered; power > 0 reported |
| S3 | Testgrenze aus dem Intranet | yes | yes: trigger "Testgrenze" in the intranet | 20 min | wait for SetChargingProfile, apply, wait until `validTo` + 30 s | profile has `validTo` ≤ 15 min ahead; box regulates to the limit (±1 %); after `validTo` the box charges at its own max again |
| S4 | Server weg während einer Grenze | yes | yes: trigger a test limit | 20 min | after a profile arrives, drop the connection and block reconnects until `validTo` + 30 s | limit still applied while offline; limit ends at `validTo` without the server; reconnect afterwards succeeds with a fresh BootNotification |
| S5 | StopTransaction mit transactionId 0 | yes | no | 60 s | send StopTransaction with transactionId 0 | server answers `Accepted` without CALLERROR and keeps the connection |
| S6 | Box lehnt Profil ab | yes | yes: trigger a test limit | 20 min | box configured to reject; wait for a profile | box answered `Rejected`; connection stays up; no limit applied |
| S6b | Box ohne Ladestand (SoC) | yes | no | 90 s | box with `supportsSoc = false`, reboot | first ChangeConfiguration (with SoC) answered `Rejected`; second one without SoC arrives |
| S7 | Zweite Verbindung derselben Kennung | yes | no | 60 s | open a second socket with the same identity | the first connection is closed by the server (code 1000) |
| S8 | Uhr der Box geht falsch | yes | no | 180 s | set clock offset −15 min, send MeterValues while charging | MeterValues still answered (`{}`); report states that the server discards these values (documented, not observable over OCPP) |
| S9 | Fahrplan steuert die Box | **no** | yes: open charging demand in the app/intranet, local `OCPP_AKTIV=aktiv` | 30 min | plugged in, wait for a profile from the schedule (not the test limit: `validTo` > 15 min is allowed) | profile has `validTo`; limit applied; profile replaced/cleared as the schedule changes |
| S10 | Falsches Passwort | **no** | no | 60 s | connect with a wrong password, 3 times | 401 each time; no hint which part was wrong; no automatic retry loop on 401 |
| S11 | App-Sicht während des Ladens | yes | needs app login | 25 min | charging at constant power ≥ 15 min, poll app view every 30 s | dashboard `verbindung` = `online`; device status not `offline`; `live.wallbox_leistung_w` within ±10 % of the box power; `geraete-energie` quarter-hour kWh within ±15 % of the box meter for that quarter |

Notes:
- S8: the effect (server discards old values) is not visible to the box; the check that can fail is only the
  protocol answer. The report says so instead of pretending to verify more.
- S11 is expected to fail today on `live.wallbox_leistung_w` and device status (gap documented in
  `CENTRAL_SYSTEM_BEHAVIOUR.md`). A failing check here is a correct finding, not a simulator bug.
- S10 must never run against live: the server's failure counter is per IP and would lock out the office.
