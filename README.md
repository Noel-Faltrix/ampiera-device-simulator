# Ampiera Device Simulator

Desktop app for macOS and Windows that simulates OCPP 1.6J wallboxes. The simulated wallboxes connect to the
Ampiera central system (local backend or `wss://api.ampiera.de/ocpp`), take charging profiles like a real box,
and run fixed test scenarios against the central system. The "App-Sicht" logs in as a test customer and shows
what the customer app receives for the same wallbox.

It tests the server against our reading of the OCPP specification. It does not replace the test with a real
wallbox before `OCPP_AKTIV` is switched on.

## What is where

| Path | Content |
|---|---|
| `crates/sim-core` | OCPP client, charge point model (status, profiles, vehicle, meter), safety rules, scenarios S1–S11 |
| `crates/app-view` | Read-only client for the customer app API (login, device code, token refresh, snapshot) |
| `src-tauri` | Desktop shell: Tauri commands and events, OS keychain, saved wallbox list, file export |
| `src` | React UI (German texts, Ampiera design tokens) |
| `docs/` | Interface contract, central system behaviour, scenarios, onboarding, app view guide |

Start with [ARCHITECTURE.md](ARCHITECTURE.md), then [docs/CONTRACT.md](docs/CONTRACT.md).

## Run locally

Requirements: Node 22, Rust (stable, via rustup), on macOS the Xcode command line tools.

```
cd ~/Documents/Github/ampiera-device-simulator && npm ci
cd ~/Documents/Github/ampiera-device-simulator && npm run tauri dev
```

To test against a local backend, start backend and worker there (`ampiera-backend` README, "Lokal starten");
the OCPP central system listens on `ws://localhost:9000/ocpp`, the API on `http://localhost:3000`.

## Test

```
cd ~/Documents/Github/ampiera-device-simulator && npm run lint && npm test && npm run build
cd ~/Documents/Github/ampiera-device-simulator && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

`npm run build` must run before the cargo steps: the shell embeds `dist/`.

## Build installers

Locally (builds for the current platform; output in `target/release/bundle/`):

```
cd ~/Documents/Github/ampiera-device-simulator && npm run tauri build
```

On GitHub, `.github/workflows/build.yml` builds on every push to `main`: checks on Linux, then a universal
macOS `.dmg` and Windows `.msi`/`setup.exe` as workflow artifacts. A tag `v*` creates a draft release with
the installers. The installers are not signed: macOS asks for confirmation once (System Settings →
Privacy & Security → "Open Anyway"), Windows shows SmartScreen ("More info" → "Run anyway").

## Credentials for a simulated wallbox

Each wallbox needs an OCPP identity and password from the backend. For the simulation installation of the
hersteller simulator (on the server, inside the backend container):

```
cd /opt/ampiera && docker compose exec backend node dist/scripts/herstellersim.js --ocpp-wallbox
```

The password is printed once. Enter it only in the simulator; it is stored in the macOS Keychain or the
Windows Credential Manager, never in a file. App view login: see [docs/APP_LIVE_ANSEHEN.md](docs/APP_LIVE_ANSEHEN.md).
