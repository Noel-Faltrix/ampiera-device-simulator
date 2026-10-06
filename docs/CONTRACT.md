# Interface contract: core ↔ desktop shell ↔ UI

Fixed before the build (06.10.2026). The Rust core (`crates/sim-core`), the Tauri shell (`src-tauri`) and the
React UI (`src`) are built in parallel against this file. Changing a name here means changing it in all three.

All JSON uses `camelCase` field names (`#[serde(rename_all = "camelCase")]`). Enums are serialized as
lower_snake strings (`#[serde(rename_all = "snake_case")]`) unless stated. Times are ISO 8601 UTC strings.
Missing measurements are `null`, never `0`.

## 1. Types

```ts
type TargetKind = "local" | "live";

/** Defaults: local = "ws://localhost:9000/ocpp", live = "wss://api.ampiera.de/ocpp". */
interface ChargePointConfig {
  label: string;                 // shown in the UI, e.g. "Box 1"
  targetKind: TargetKind;
  baseUrl: string;               // without trailing slash; identity is appended: <baseUrl>/<identity>
  identity: string;              // OCPP "Kennung", e.g. "AP7K2M9QX4RT"
  vendor: string;                // BootNotification chargePointVendor, max 20 chars
  model: string;                 // BootNotification chargePointModel, max 20 chars
  phases: 1 | 3;
  maxPowerW: number;             // hardware limit of the simulated box (e.g. 11000)
  supportsSoc: boolean;          // false -> rejects MeterValuesSampledData containing SoC
  acceptedRateUnits: ("W" | "A")[];   // profile units the box accepts; others are answered "Rejected"
  rejectProfiles: boolean;       // answer every SetChargingProfile with "Rejected"
  clockOffsetS: number;          // simulated clock error of the box, seconds (S8)
}

interface VehicleConfig {
  capacityKwh: number;           // usable battery capacity
  socPct: number;                // state of charge when plugged in, 0..100
  maxPowerW: number;             // on-board charger limit
  phases: 1 | 3;
}

type ConnectionState =
  | { state: "disconnected" }
  | { state: "connecting" }
  | { state: "connected"; since: string }
  | { state: "reconnecting"; attempt: number; nextAttemptAt: string }
  | { state: "failed"; reason: string };           // e.g. 401, TLS error; no automatic retry on 401

type OcppStatus = "Available" | "Preparing" | "Charging" | "SuspendedEV" | "SuspendedEVSE"
  | "Finishing" | "Reserved" | "Unavailable" | "Faulted";

interface ActiveLimit {
  limitW: number;                // effective limit converted to W (A * 230 * phases)
  rawLimit: number;              // as received
  rateUnit: "W" | "A";
  profileId: number;
  purpose: "ChargePointMaxProfile" | "TxDefaultProfile" | "TxProfile";
  validTo: string | null;
}

interface ChargePointSnapshot {
  id: string;                    // local id (uuid), not the OCPP identity
  config: ChargePointConfig;
  connection: ConnectionState;
  status: OcppStatus;
  vehicle: { config: VehicleConfig; socPct: number; plugged: boolean } | null;
  powerW: number | null;         // current charging power; null when unknown
  energyWh: number;              // meter register of the box (Energy.Active.Import.Register)
  transactionId: number | null;
  activeLimit: ActiveLimit | null;   // null = no profile in force, box charges at its own max
  profileCount: number;
  heartbeatIntervalS: number | null; // from BootNotification response
  lastError: string | null;
  updatedAt: string;             // time of the last state change of this box ("Stand" in the UI)
}

type FrameDirection = "out" | "in";   // out = box -> central system
interface FrameLogEntry {
  chargePointId: string;
  at: string;
  direction: FrameDirection;
  raw: string;                   // OCPP-J frame text; never contains credentials
}

type ScenarioId = "S1" | "S2" | "S3" | "S4" | "S5" | "S6" | "S6b" | "S7" | "S8" | "S9" | "S10" | "S11";
interface ScenarioInfo {
  id: ScenarioId;
  title: string;                 // German, shown in the UI
  description: string;           // German, what happens and what a human must do (if anything)
  liveAllowed: boolean;          // false for S9, S10
  needsHuman: boolean;           // e.g. S3: trigger the test limit in the intranet
  timeoutS: number;
}
type CheckOutcome = "passed" | "failed" | "skipped";
interface CheckResult { name: string; outcome: CheckOutcome; detail: string }   // German text
interface ScenarioReport {
  scenarioId: ScenarioId;
  chargePointId: string;
  targetKind: TargetKind;
  startedAt: string;
  finishedAt: string;
  outcome: "passed" | "failed" | "aborted";
  checks: CheckResult[];
  chargePointLabel?: string;     // for the report header; absent in old reports
  chargePointIdentity?: string;
}

// ── App view (plan section 4a) ──
type AppLoginResult = { result: "ok" } | { result: "device_code_required" };
interface AppViewSnapshot {
  fetchedAt: string;
  installationId: string | null;
  summary: {
    connection: string | null;        // dashboard anlagen[].verbindung
    lastContact: string | null;       // dashboard anlagen[].letzter_kontakt
    deviceStatus: string | null;      // dashboard geraete[] (typ wallbox).status
    livePowerW: number | null;        // dashboard live.wallbox_leistung_w
    geoState: string | null;          // anlagenlage geraete[] wallbox .zustand
    geoHeadline: string | null;       // anlagenlage .ueberschrift
    lastQuarterKwh: number | null;    // geraete-energie latest wallbox value kwhPositiv
    lastQuarterAt: string | null;
    nextScheduleSteps: { start: string; action: string; targetPowerW: number }[];
  };
  raw: Record<string, unknown>;       // endpoint path -> parsed JSON body (or {error})
}
```

## 2. Tauri commands (`invoke`)

| Command | Args | Returns |
|---|---|---|
| `list_charge_points` | – | `ChargePointSnapshot[]` |
| `add_charge_point` | `{ config: ChargePointConfig, password: string, liveConfirmed: boolean }` | `string` (id) |
| `remove_charge_point` | `{ id }` | – |
| `connect` / `disconnect` | `{ id }` | – |
| `plug_in` | `{ id, vehicle: VehicleConfig }` | – |
| `unplug` | `{ id }` | – |
| `reboot` | `{ id }` | – (closes and reconnects, sends BootNotification again) |
| `list_scenarios` | – | `ScenarioInfo[]` |
| `run_scenario` | `{ id, scenarioId }` | `ScenarioReport` |
| `abort_scenario` | `{ id }` | – |
| `export_log` | `{ id }` | `string` (JSON array of `FrameLogEntry`) |
| `export_report` | `{ report: ScenarioReport }` | `string` (Markdown) |
| `save_log` | `{ id }` | `string` (absolute path of the written JSON file in the user's Downloads folder) |
| `save_report` | `{ report: ScenarioReport }` | `string` (absolute path of the written Markdown file in Downloads) |
| `app_redeem_invite` | `{ baseUrl, email, inviteToken, password }` | – |
| `app_login` | `{ baseUrl, email, password }` | `AppLoginResult` |
| `app_verify_device` | `{ code }` | – |
| `app_logout` | – | – |
| `app_snapshot` | – | `AppViewSnapshot` |

App `baseUrl` defaults: local `http://localhost:3000`, live `https://api.ampiera.de`. Paths used (read only,
via the `/api/app/v1` aliases): `GET /dashboard`, `GET /evaluation/:id/geo-position`, `GET /schedule/:id`,
`GET /device-energy?von=&bis=&aufloesung=viertelstunde&anlageId=`.

Errors: every command returns `Result<T, String>`; the string is German, names the cause, never contains a
password or token.

## 3. Events (Rust → UI)

| Event | Payload |
|---|---|
| `charge-point-updated` | `ChargePointSnapshot` (emitted on every state change, at most 4 per second per box) |
| `frame-logged` | `FrameLogEntry` |
| `charge-point-removed` | `{ id }` |
| `restore-problem` | `{ message: string }` (German). A saved wallbox could not be restored at startup (password missing in the keychain, rejected by the core, damaged `boxes.json`). Delivered on the first `list_charge_points` call, one event per problem — the UI must subscribe before it lists. |

## 4. Fixed rules the core enforces (not the UI)

- The effective target is derived from the URL host, not trusted from the UI: every host that is not
  localhost/loopback/private IPv4 counts as live. `targetKind = "local"` with such a host is rejected.
  Restored boxes are checked the same way.
- Two boxes with the same normalized `baseUrl` + `identity` are rejected (they would push each other out).
- Live: `add_charge_point` with `targetKind = "live"` fails unless `liveConfirmed = true`; at most 3 live boxes
  connected at the same time; `ws://` only for `localhost`, `127.0.0.1`, `::1` and private IPv4 ranges;
  certificate validation always on.
- Scenarios with `liveAllowed = false` fail immediately on a live box. The live limit applies to every
  connect path, scenarios included.
- After HTTP 401 or 429 a box refuses new connection attempts for 60 s (protects the server's per-IP
  failure counter: 20 per 15 min).
- Input bounds: label ≤ 60 chars, vendor/model ≤ 20, no control characters; password 1–200 printable ASCII;
  maxPowerW and vehicle maxPowerW 1–350000; capacityKwh 1–300; clockOffsetS within ±86400; at most 20
  boxes. baseUrl without userinfo, query or fragment.
- Server-controlled values are clamped: heartbeat and meter interval ≥ 5 s; at most 50 profiles with 100
  periods each; queued outgoing calls ≤ 100; incoming WebSocket messages ≤ 64 KiB; logged frames are cut
  at 16 KiB.
- OCPP password and app password/tokens live only in the OS keychain (service `de.ampiera.device-simulator`)
  and in memory; the Basic-Auth header and tokens are never logged or exported.
- App polling at most every 30 s (`app_snapshot` returns the cached result if called sooner).

## 5. Rust API of `sim-core` used by the shell

```rust
// crates/sim-core/src/lib.rs
pub mod model;      // all types of section 1 (serde, camelCase) except the app view types

/// Receives state changes; the shell forwards them as Tauri events.
pub trait EventSink: Send + Sync + 'static {
    fn charge_point_updated(&self, snapshot: &model::ChargePointSnapshot);
    fn frame_logged(&self, entry: &model::FrameLogEntry);
    fn charge_point_removed(&self, id: &str);
}

/// Owns all simulated charge points. Cheap to clone (Arc inside). All methods are async and
/// return Result<_, SimError> where SimError: std::error::Error + Display (German message).
pub struct Simulator { /* … */ }
impl Simulator {
    pub fn new(sink: std::sync::Arc<dyn EventSink>) -> Self;
    pub async fn list(&self) -> Vec<model::ChargePointSnapshot>;
    pub async fn add(&self, config: model::ChargePointConfig, password: String, live_confirmed: bool) -> Result<String, SimError>;
    pub async fn remove(&self, id: &str) -> Result<(), SimError>;
    pub async fn connect(&self, id: &str) -> Result<(), SimError>;
    pub async fn disconnect(&self, id: &str) -> Result<(), SimError>;
    pub async fn plug_in(&self, id: &str, vehicle: model::VehicleConfig) -> Result<(), SimError>;
    pub async fn unplug(&self, id: &str) -> Result<(), SimError>;
    pub async fn reboot(&self, id: &str) -> Result<(), SimError>;
    pub fn scenarios(&self) -> Vec<model::ScenarioInfo>;
    pub async fn run_scenario(&self, id: &str, scenario: model::ScenarioId) -> Result<model::ScenarioReport, SimError>;
    pub async fn abort_scenario(&self, id: &str) -> Result<(), SimError>;
    pub async fn export_log(&self, id: &str) -> Result<String, SimError>;
}
pub fn report_to_markdown(report: &model::ScenarioReport) -> String;

// crates/app-view/src/lib.rs  (separate crate `app_view`, depends on nothing in sim-core)
pub struct AppClient { /* holds tokens in memory only */ }
impl AppClient {
    pub fn new() -> Self;
    pub async fn redeem_invite(&self, base_url: &str, email: &str, invite_token: &str, password: &str) -> Result<(), AppError>;
    pub async fn login(&self, base_url: &str, email: &str, password: &str, device_id: &str) -> Result<AppLoginResult, AppError>;
    pub async fn verify_device(&self, code: &str) -> Result<(), AppError>;
    pub async fn logout(&self) -> Result<(), AppError>;
    pub async fn snapshot(&self) -> Result<AppViewSnapshot, AppError>;   // cached for 30 s
}
```

The shell stores the OCPP password in the keychain on `add_charge_point` (account = local id) and passes it
to `Simulator::add`; on app start it re-creates saved boxes from `boxes.json` in the app config directory
(config and the `liveConfirmed` flag only, never passwords) and reads passwords back from the keychain. The app `device_id` is a random
id created once and kept in the app config directory.
