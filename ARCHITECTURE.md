# Architecture

```
React UI (src/)  ──invoke/listen──►  Tauri shell (src-tauri/)  ──►  sim-core::Simulator ──OCPP-J/WebSocket──► central system
                                          │                                   │
                                          │                                   └─ scenarios (S1–S11) use the same boxes
                                          ├─► OS keychain (OCPP passwords)
                                          ├─► boxes.json (config only, app config dir)
                                          └─► app_view::AppClient ──HTTPS──► customer app API (read only)
```

The three layers are built against one contract: [docs/CONTRACT.md](docs/CONTRACT.md). A name changed there
is changed in all three.

## sim-core

- `Simulator` owns all boxes. Each box is an actor task (`ocpp/actor/`) that owns its WebSocket, its
  state and a 1-s simulation tick. Commands reach it through `BoxHandle`; state leaves it as snapshots and
  frame log entries through `EventSink` (throttled to 4 updates per second per box).
- `charge_point/` is pure logic without IO: status derivation, charging profile store and evaluation,
  vehicle and meter, answers to calls from the central system. Everything there is tested without a network.
- `policy.rs` and `gate.rs` hold the rules from CONTRACT section 4 that the core enforces no matter what the
  UI sends: live is derived from the URL host, live needs confirmation, at most 3 live wallboxes online,
  no duplicate wallbox, input bounds, 60-s cooldown after 401/429. Every connect path goes through the gate,
  scenarios included.
- `scenarios/` runs S1–S11 on an existing box and returns a report of checks. Pure comparison rules live in
  `scenarios/rules.rs` and `scenarios/app.rs`.
- Charging limits end at `validTo` on the box itself, also while it is offline. This mirrors rule §8 of the
  Ampiera main rule set ("a command ends on the device").

## app-view

`AppClient` holds tokens in memory only, rotates refresh tokens under one lock, refuses redirects, caps
response size and caches every snapshot result (success or failure) for 30 s. `extract.rs` turns the four
endpoint bodies into the summary; missing values stay `null`.

## src-tauri

Commands map 1:1 to CONTRACT section 2. Passwords go to the keychain (service `de.ampiera.device-simulator`),
`boxes.json` holds configs and the live confirmation flag. Restored wallboxes start disconnected and are
validated by the core again. `save_log` / `save_report` write into the Downloads folder. Capabilities: only
`core:event:default`; strict CSP without inline styles.

## UI

All Tauri calls are in `src/api/client.ts`. State is a reducer in `src/state/store.tsx`, fed by events.
Colours, spacing and fonts come only from `src/styles/tokens.css` (copy of the intranet tokens).

## Known gaps of the central system

See [docs/CENTRAL_SYSTEM_BEHAVIOUR.md](docs/CENTRAL_SYSTEM_BEHAVIOUR.md), last section: the customer app does
not see live power or status of an OCPP-only wallbox. Scenario S11 reports this.
