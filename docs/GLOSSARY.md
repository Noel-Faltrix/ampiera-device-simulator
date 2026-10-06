# Code Glossary (German → English)

Source: ampiera-brain/03 Konventionen – Entwicklung/03.20 Code-Glossar Deutsch-Englisch.md. Change it there first, then copy.

Status: accepted by Noel (CTO), 05.10.2026. Valid in all Ampiera repos (backend, admin dashboard, app, weather service). Never invent your own translation: if a term is missing, add it to the glossary first.

Terms come from the real code (Prisma schema with 116 models and 79 enums, `backend/src`, `ampiera-admin-dashboard/src`, `ampiera-app/lib`, `ampiera-wetter/src`). Counts in the notes are approximate hit counts across all repos.

---

## 1. Rules

1. **Casing:** functions and variables `camelCase`, types/classes/models/enums/widgets `PascalCase`, constants `UPPER_SNAKE_CASE`. Files: TS `camelCase`, Dart and Python `snake_case`, React components `PascalCase`. Prisma fields `camelCase`, Prisma enum names `PascalCase`.
2. **No umlaut transcription.** Old code writes `ae/oe/ue/ss` (`Verguetung`, `Ueberschuss`, `Schluessel`). English names do not have the problem. The "English" column is authoritative, never a back-transcription.
3. **Boolean prefixes:** `ist…` → `is…`, `hat…` → `has…`, `darf…`/`kann…` → `can…` (`darfVerarbeiten` → `canProcess`), `soll…` → `should…`. Negation only as `not`/`no` in constants, never `isNot…` in function names.
4. **One word per meaning; verbs in section 3.** Core rule: `get` = pure in-memory access, `load` = from our own database or file, `fetch` = over the network from a third-party system, `read` = extract from a value you were handed (body, JSON, file content), `find` = look up, result may be `null`, `list` = many. `check` = verify a business rule, `validate` = verify input/format, `verify` = verify token, code, signature, identity.
5. **Timestamp patterns:** `…Am` → `…At` (`erstelltAm` → `createdAt`), `…Von` → `…By` (`erstelltVon` → `createdBy`), `…Bis` → `…Until`/`…To`, `…Ab` → `…From`/`…Since`. Units stay as suffix (`Ms`, `W`, `Kwh`, `Pct`, `CtKwh`). A missing measurement stays `null`, never `0`.
6. **Legal terms:** if the term has an established English technical term (EDI@Energy, ENTSO-E, EU directive), use it (`MeteringLocation`, `MarketLocation`, `GatewayAdministrator`, `MeteringPointOperator`, `GridOperator`, `FeedInTariff`). Names of laws and registers stay as **foreign names** in normal casing, explained in the doc comment: `paragraph14a`, `wim`, `mastr`, `obis`, `eeg`, `msbg`, `enwg`. Abbreviations (MaLo, MeLo, MSB, GWA) are **never** names, only allowed in comments; code uses the spelled-out form (`marketLocationId`).
7. **The database stays unchanged:** every renamed model gets `@@map("old_table_name")`, every field `@map("old_column")`, every renamed enum value `@map("old_value")`. `prisma migrate diff` must be empty. Fields that had the same name as their column (e.g. `bezeichnung`, `strasse`, `messlokation`, `zeit`, `kwh`) newly get an explicit `@map("bezeichnung")`. Enum type names are translated in Block E; enum values follow with Block 7a's new API; old paths keep the old values through the `…AlsJson` converters (e.g. `anlageAlsJson`), which keep the old field names.
8. **User-facing text stays German** (app, intranet, mails, PDFs, help wiki, error messages shown to users), with umlauts. Keys and names around it are English (`errors.notFound`), the text is German. Wallbox stays "Wallbox" in user text. Comments, test titles (`describe`/`it`) and commit messages are English.

---

## 2. Domain terms

"German (as in code)" is the word as it appears in the code (word stem; compounds are translated word by word: `anlageStillgelegt` = `installation` + `decommissioned`). Type and model names in PascalCase, stems in camelCase.

### 2.1 Customer and contract

| German (as in code) | English | Note |
|---|---|---|
| Kunde (`Kunde`, model) | `Customer` | Routes/files partly English already (`customers.ts`, `requireCustomerAuth`) |
| Kundennummer | `customerNumber` | |
| Kundentyp | `customerType` | |
| KundeStatus | `CustomerStatus` | Values see 2.12 |
| pilot / aktiv / pausiert / gekuendigt | `pilot` / `active` / `paused` / `terminated` | Enum values, `@map` |
| Vorname / Nachname | `firstName` / `lastName` | |
| Anschrift / Adresse | `address` | One word for both |
| Strasse / Hausnummer / PLZ / Ort | `street` / `houseNumber` / `postalCode` / `city` | |
| Telefon | `phone` | |
| Vertrag / Verträge | `contract` / `contracts` | `vertraege_screen.dart` → `contracts_screen.dart` |
| Kündigung | `termination` | |
| Startangaben | `initialData` | Data given at the start (staff or customer); `AnlageStartangaben` → `InstallationInitialData` |
| Kundenlink | `CustomerLink` | Link the customer uses to submit data |
| KundenlinkModus neu/ergaenzen | `create` / `amend` | |
| Kundenlink-Kanal email/sms | `email` / `sms` | |
| Einreichung | `submission` | `KundenEinreichung` → `CustomerSubmission` |
| Eingang (intranet list) | `inboxItem` | Eingang = entry in the inbox, Einreichung = what the customer sends |
| Link erstellen (Dialog, Plan 6) | `createLink` | Staff dialog and function that creates a link of any kind: `customer`, `fitting`, `partner`, `landlord` (`LinkKind`); route `POST /links`; hilfe key `link-erstellen` |
| externer Link (Partner oder Vermieter, Plan 6) | `externalLink` / `ExternalLink` | Link for a person who is not a customer; module `services/externalLink/`, tables `external_links`, `external_submissions`, `external_submission_files`, `external_link_log`; only the SHA-256 of the key is stored; switch `KUNDENDATEN_ENC_KEY` |
| Partnerlink | `partnerLink` | `ExternalLink` of kind `partner`: a partner company registers itself; page `/partnerlink#<key>`, public path `partner-link` |
| Vermieterlink | `landlordLink` | `ExternalLink` of kind `landlord`: the owner of the rented property consents to the fitting; page `/vermieterlink#<key>`, public path `landlord-link`; created only with `customerInformed` |
| externe Einreichung | `ExternalSubmission` | What the recipient of an external link entered; status `draft` / `submitted` / `accepted`; shown in the inbox with `kind` `partner` or `landlord` |
| Link zurückziehen | `revokeLink` | Staff takes a link back (`revokedAt`); a submitted entry stays. Not `withdrawal`, which is the customer taking back a consent |
| Link erneut senden | `resendLink` | New link with the same parameters, old one revoked, mail sent (only the hash is stored, so the old key cannot be sent again) |
| Zustand eines Links | `LinkState` | `created` / `opened` / `started` / `completed` / `expired` / `revoked` / `installation_decommissioned`; computed, never stored |
| Zwischenstand (Seite) | `draft` | Saved on the server at each "Weiter"; `ExternalSubmission.draft` |
| Empfänger (eines Links) | `recipient` | `recipientEmail`, `recipientName`; normalised e-mail (trimmed, lower case) |
| Objekthinweis | `objectHint` | Short text staff add for the landlord, at most 80 characters, no names |
| Kunde ist einverstanden (Vermieterlink) | `customerInformed` | The customer agreed that the landlord is contacted; mandatory for a landlord link |
| Rechtsform | `legalForm` | Of a partner company; column `partner.legal_form` |
| Elektrofachkraft nach DIN VDE 1000-10 | `qualifiedElectrician` | Column `partner.qualified_electrician`; NULL = not recorded |
| Gewerbenachweis / Qualifikationsnachweis | `tradeProof` / `qualificationProof` | PDF purposes `trade_proof` / `qualification_proof` of a partner |
| unterschriebene Zustimmung (Vermieter) | `signedConsent` | PDF purpose `signed_consent` of a landlord submission |
| Partner-Bedingungen / Meldungsverarbeitung | `partnerTerms` / `reportProcessing` | Texts a partner accepts (`PartnerAcceptance.kind` `partner_terms` / `report_processing`); a draft text carries `isDraftText` |
| Partner-Dokument / Partner-Zustimmung | `PartnerDocument` / `PartnerAcceptance` | Interim proof storage until block 4; record of an acceptance (version, time, source), never updated by code |
| Kundendokument | `CustomerDocument` | |
| Kundendokument-Art: Typenschild / Stromrechnung / Installationsfoto / Messprotokoll | `nameplate` / `electricityBill` / `fittingPhoto` / `measurementRecord` | `Installationsfoto` refers to the fitting → `fitting` |
| Anschlussnehmer | `ConnectionOwner` | Legal term (NAV), owner of the grid connection; pitfall 8 |
| Anschlussnutzer | `ConnectionUser` | Usually the tenant |
| Anschlussnutzer-Wechsel | `ConnectionUserChange` | |
| Mietobjekt | `RentalUnit` | |
| Vermieter / Mieter | `landlord` / `tenant` | |
| Partner | `partner` | Fitting/support partner |
| PartnerRolle einbau / betreuung | `fitting` / `support` | |
| Partner-Meldeweg | `reportChannel` | |
| Pseudonym | `pseudonym` | |
| Kundenakte (Plan 9.3) | `customerRecord` | What staff open about one customer; access log view `Kundenakte`; the timeline is a view of it |
| Zeitstrahl (Plan 9.3) | `timeline` | Newest-first list of what happened to one customer, merged from several sources; route `customers/:id/timeline`, access log view `Zeitstrahl` |
| Zeitstrahl-Eintrag (Plan 9.3) | `TimelineEntry` | One line of the timeline: `id` (`type:rowId`), `type` (`TimelineType`), `at`, `title`, optional `detail` and `link` |

### 2.2 Installation and devices

| German (as in code) | English | Note |
|---|---|---|
| Anlage (`Anlage`, approx. 8000 hits) | `Installation` | The home energy system; the on-site activity is `Fitting` (decision 6) |
| Anlagen (plural, lists) | `installations` | |
| AnlagePartner | `InstallationPartner` | |
| Anlagenwächter | `InstallationWatchdog` | Detects faults / under-performance |
| Anlagenlage | `InstallationGeoPosition` | Pitfall 5 (Lage) |
| Bezeichnung | `label` | Display name; `name` stays `name` |
| Standort | `location` | Place of the installation; pseudonym at the weather service: `locationId` |
| Lage / Lagegenauigkeit / Lagequelle | `geoPosition` / `geoPositionAccuracy` / `geoPositionSource` | lat/lon plus accuracy (`haus`/`strasse`/`plz` → `house`/`street`/`postalCode`) |
| Dachfläche | `RoofSurface` | |
| Azimut / Neigung | `azimuth` / `tilt` | |
| Luftbild | `aerialImage` | |
| Gerät (`Geraet`, approx. 3300) | `Device` | |
| Geräte (plural, also name part "geraete…") | `devices` | |
| GeraetTyp | `DeviceType` | |
| pv_wechselrichter | `pv_inverter` | |
| batteriespeicher | `battery_storage` | |
| wallbox | `ev_charger` | Decision 2; `evCharger` throughout the code |
| waermepumpe | `heat_pump` | |
| heizstab | `immersion_heater` | Plan: `planImmersionHeater` |
| klimageraet | `air_conditioner` | |
| GeraetStatus ok/offline/fehler | `ok` / `offline` / `error` | |
| Wechselrichter | `inverter` | |
| PV (installation, power, day) | `pv` | Stays `pv`; kWp → `Kwp` |
| Speicher (battery) | `battery` | Pitfall 9; not `storage` |
| Wallbox | `evCharger` | `chargePoint` only in the OCPP part (`OcppChargePoint`) |
| Wärmepumpe (`Waermepumpe`, short `wp`) | `heatPump` (short `hp`) | |
| Heizstab | `immersionHeater` | |
| Klimagerät | `airConditioner` | |
| Zähler | `meter` | |
| Steuerbox | `ControlBox` | VDE FNN "Steuerbox", in prose `control box` |
| Kennwerte (`SpeicherKennwerte` etc.) | `specs` (`BatterySpecs`, `EvChargerSpecs`, `HeatPumpSpecs`, `ImmersionHeaterSpecs`) | Nameplate values such as capacity and power |
| Kennwertquelle aggregator/formular/geraet | `SpecSource` `aggregator`/`form`/`device` | |
| Fähigkeit(en) | `capability` / `capabilities` | |
| Hersteller | `manufacturer` | |
| Hersteller-Cloud | `manufacturerCloud` | |
| Seriennummer | `serialNumber` | |
| Modell (device model) | `deviceModel` | Never `model` alone (Prisma keyword / confusion) |
| Zugang (device account at the manufacturer) | `DeviceAccount` | `GeraeteZugang` → `DeviceAccount`; pitfall 10 |
| ZugangAnbieter | `AccountProvider` | `enode`, `goe_cloud` etc. stay (brand names) |
| Anbindung | `integration` | |
| Anbindungsweg | `IntegrationMethod` | aggregator/adapter/keiner/hersteller_cloud/ocpp/simulator → `aggregator`/`adapter`/`none`/`manufacturer_cloud`/`ocpp`/`simulator` |
| Adapter (meter cabinet) | `adapter` | |
| Einbau (on-site fitting) | `fitting` | Decision 6; `EinbauLink` → `FittingLink`, `EinbauHtml` → `FittingHtml` |
| Einbauprotokoll | `FittingRecord` | Document, not log (pitfall 12) |
| EinbauprotokollArt einbau/tausch/ausbau/pruefung | `fit` / `swap` / `remove` / `inspection` | |
| Einbau-Check | `FittingCheck` | Check points `geraete_verbunden` → `devices_connected`, `einspeisebegrenzung` → `feed_in_limit`, `steuerbox` → `control_box`, `internet` |
| Zählerschrank | `meterCabinet` | |
| Messstellen-Gerät | `MeteringPointDevice` | `MessstellenGeraet`; kind zaehler/gateway/steuerbox/kommunikationseinheit → `meter`/`gateway`/`control_box`/`communication_unit` |
| Ladebedarf | `ChargingNeed` | Energy wanted by departure |
| Abfahrt | `departure` | |
| Ladefrist | `chargeDeadline` | |
| Ladestand | `soc` | State of charge, already `soc` in code |
| Sofortladung | `InstantCharge` | Pitfall 16; mode `dauer`/`bis_voll` → `duration`/`until_full` |
| Ladevorgang | `ChargingSession` | OCPP |
| OcppStation | `OcppChargePoint` | "Charge Point" is the OCPP term |
| Ladeleistung / Entladeleistung | `chargePower` / `dischargePower` | |
| Laden / Entladen / Halten (action) | `charge` / `discharge` / `hold` | `FahrplanAktion` |
| Netzladen / Netzladesperre | `gridCharging` / `gridChargingLock` | Charging the battery from the grid |
| Warmwasser / Zapffenster | `hotWater` / `hotWaterWindow` | Time windows in which hot water is drawn |
| Legionellen(-Schutz) | `legionella` | `legionellaWeekday` |
| Taktung (heat pump/battery) | `shortCycling` | **not** `tick`; pitfall 13 |
| Verschleiß | `wear` | `wearSurcharge` |
| SG Ready | `sgReady` | Standard name, stays |
| Relais | `relay` | |
| Simulator / Welt / Lastgang | `simulator` / `world` / `loadProfile` | `SimulatorWelt` → `SimulatorWorld` |
| Schattenlastgang | `shadowLoadProfile` | |

### 2.3 Metering, meters, metering point operator

| German (as in code) | English | Note |
|---|---|---|
| Messstelle | `MeteringPoint` | Plan: the whole of meter-cabinet devices and fitting (MsbG "Messstelle") |
| Messlokation (`messlokation`, "MeLo") | `meteringLocation` | EDI@Energy term; stored as ID → `meteringLocationId`; = Zählpunkt (pitfall 1) |
| Marktlokation ("MaLo") | `marketLocation` / `marketLocationId` | 11 digits with check digit |
| Zählpunkt | `meteringLocation` | Synonym, **no** own name in code |
| Messstellenbetreiber ("MSB", `msb`, approx. 340) | `MeteringPointOperator` | Abbreviation `mpo` only locally; spelled out in type and file names |
| Gateway-Administrator ("GWA", `gwa`) | `GatewayAdministrator` | |
| Messstellenbetreibergesetz | `msbg` | Law citation, foreign name (`msbgSection61`) |
| Messwert(e) | `measurement` / `measurements` | General measured value |
| Zählerwert (model `Zaehlerwert`) | `MeterValue` | Energy **per quarter hour**, not a meter reading |
| Zählerstand | `meterReading` | Cumulative reading; pitfall 2 |
| Viertelstunde | `quarterHour` | |
| Minutenwert | `minuteValue` | |
| Intervallwerte | `intervalValues` | |
| Telemetrie | `telemetry` | |
| OBIS-Kennzahl | `obis` | Foreign name / standard |
| Ersatzwert / wahr (ZaehlerwertStatus) | `substitute` / `actual` | "wahr" = "true value" of market communication, **not** `true` |
| ZaehlerwertQuelle gwa/simulator | `gateway_administrator` / `simulator` | |
| Eichfrist | `calibrationPeriod` | Statutory re-calibration period, 8 years |
| Eichjahr | `calibrationYear` | |
| Eichlos / Gerätelos | `calibrationLot` / `deviceLot` | Lot of identical meters (MessEV § 35); confirm the meaning with André before renaming |
| Messkonzept | `meteringConcept` | |
| Verbrauch (MSB consumption, retrieval) | `consumption` | Quantity in kWh |
| MsbAbruf | `MeteringDataRequest` | "Retrieval on request" per MsbG § 61; status angefordert/verspaetet/geliefert/ausgeblieben/nicht_moeglich → `requested`/`late`/`delivered`/`missing`/`not_possible` |
| Marktkommunikation (`MaKo`) | `marketCommunication` | |
| MarktVorgang | `MarketProcess` | Status offen/clearing/erledigt/abgebrochen → `open`/`clearing`/`done`/`cancelled` |
| MarktprozessArt | `MarketProcessKind` | Values stay as foreign names with an English part (`msb_beginn_anmeldung` → `mpo_start_registration`) |
| WiM | `wim` | "Wechselprozesse im Messwesen", foreign name |
| MaStR / MaStR-Nummer | `mastr` / `mastrNumber` | Marktstammdatenregister, foreign name |
| Frist / Fristen | `deadline` / `deadlines` | |
| Werktag (market communication) | `businessDay` | |
| Vorgänger-MSB | `predecessorMeteringPointOperator` | Spelled out; `predecessorMpo` only locally |
| Anmeldung beim Netzbetreiber | `registration` | Pitfall 14 (≠ login) |
| Onboarding | `onboarding` | |
| OnboardingSchritt | `OnboardingStep` | Values `anlass_uebergabe` → `handover_trigger`, `kontaktweitergabe` → `contact_forwarding` |

### 2.4 Schedule and optimisation

| German (as in code) | English | Note |
|---|---|---|
| Fahrplan (approx. 1840) | `Schedule` | Plan decision; the verb is `plan` (section 3) |
| Fahrplanschritt | `ScheduleStep` | |
| Slot | `slot` | Stays |
| Planungsschritt / Schritt | `step` | |
| Optimierung / Optimierungsziel | `optimization` / `OptimizationGoal` | `kosten`/`eigenverbrauch` → `cost`/`self_consumption` |
| Empfehlung | `Recommendation` | |
| Entscheidung / Kontext | `Decision` / `DecisionContext` | |
| Eigenverbrauch | `selfConsumption` | |
| Überschuss | `surplus` | PV surplus: `pvSurplus` |
| Hauslast | `houseLoad` | Power in W |
| Hausverbrauch | `houseConsumption` | Energy in kWh |
| Gesamtpreis (per slot) | `totalPrice` | |
| Preisreihe | `priceSeries` | |
| Preishorizont | `priceHorizon` | |
| Kosten / Ersparnis | `cost` / `savings` | |
| Referenztag | `referenceDay` | Day without automation, comparison basis |
| Betriebsmodus | `OperatingMode` | beobachtung/aktiv/wartung → `observation`/`active`/`maintenance` |
| Wirkung (model `WirkungTag`) | `impact` / `ImpactDay` | Pitfall 6 (≠ Wirkungsgrad) |
| Wirkungsgrad | `efficiency` | Charge/discharge efficiency: `roundTripEfficiency` |
| Wirkungsnachweis | `impactReport` | PDF proof |
| Lerndaten | `learningData` | |
| Lernschleife | `learningLoop` | |
| Güte | `quality` | Always `quality` (`DataQuality`, `ForecastQuality`) |
| Datengüte offen/brauchbar/lueckenhaft/unbrauchbar | `DataQuality` `open`/`usable`/`gappy`/`unusable` | |
| Konfidenz niedrig/mittel/hoch | `Confidence` `low`/`medium`/`high` | |
| Annahme(n) | `assumption(s)` | Named constants |
| Kipppunkt / Schwelle | `threshold` | |
| Regel / Regeln (files `…Regeln.ts`) | `rule` / `rules` | File name `xyzRules.ts` |
| Rechner / Kern | `calculator` / `core` | `rechner.ts` → `calculator.ts` |
| Takt | `tick` | Scheduler cycle; pitfall 13 |
| Scheduler | `scheduler` | Already English |
| Ausführung / Ausführungstor | `execution` / `executionGate` | |
| Neuplanung / Sofort-Neuplanung | `replanning` / `immediateReplanning` | |
| Lauf | `run` | |
| Auswertung | `evaluation` | The customer-facing "Auswertung" stays German text in the product |
| Szenario / Vergleich | `scenario` / `comparison` | |
| Testanlage | `testInstallation` | |
| Wirtschaftlichkeit | `profitability` | `WirtschaftlichkeitVersion` → `ProfitabilityVersion` |

### 2.5 Command path and control

| German (as in code) | English | Note |
|---|---|---|
| Befehl | `command` | Plan: `readCommand` |
| Geräte-Befehl | `DeviceCommand` | Status geplant/gesendet/bestaetigt/abgelehnt/fehlgeschlagen/verworfen → `planned`/`sent`/`confirmed`/`rejected`/`failed`/`discarded` |
| Befehlsweg (folder `befehlsweg/`) | `commandPipeline` | Chain tick → execution → confirmation → retry |
| Befehlsprotokoll | `commandLog` | Log, not document |
| Bestätigung | `confirmation` | Device accepted the command |
| Quittung | `acknowledgement` | ACK/NACK of the control box; pitfall 15 |
| Wiederholung | `retry` | Not `repeat` |
| Wächter (backup, confirmation, manufacturer) | `watchdog` | Already in code: `watchdogService` |
| Sperre / gesperrt | `lock` / `locked` | Account, link, mode; business-rule block of a rule: `block` |
| Freigabe / freigegeben | `approval` / `approved` | Pitfall 17 |
| Übersteuert | `overridden` | Decision outcome |
| Schaltung(en) | `switching` / `switchingEvents` | |
| Schalter (`.env`) | `featureFlag` | Plan: flag behind flag for real devices |
| Steuerungsart direkt/ems | `ControlMode` `direct`/`ems` | § 14a Anlage 1 No. 4.4 |
| Steuerbarkeit | `controllability` | |
| Steuerbox-Meldung | `controlBoxMessage` | |
| Begrenzen / Verstärken / Normal | `limit` / `boost` / `normal` | Heat-pump states per SG Ready |
| Drossel / Drosselung | `throttle` / `throttling` | Power reduction of the **consumer** (§ 14a); pitfall 4 |
| Abregelung | `curtailment` | Reduction of **feed-in** (PV) |
| Aggregator | `aggregator` | |
| Hersteller-Cloud-Sender | `manufacturerCloudSender` | |
| Sender (command) | `sender` | |
| Station / Ladeeinheit A/W | `chargePoint` / `unit` `A`/`W` | `chargePoint` only in the OCPP part |
| Kompatibilität | `compatibility` | `KompatibilitaetStufe` gesteuert/nur_gemessen/in_pruefung/nicht_unterstuetzt → `controlled`/`measured_only`/`in_review`/`not_supported` |

### 2.6 Grid and § 14a

| German (as in code) | English | Note |
|---|---|---|
| Netz | `grid` | |
| Netzbetreiber (approx. 400) | `GridOperator` | = distribution grid operator; **not** MSB; pitfall 3 |
| Netzbetreiber-Eingriff | `GridOperatorIntervention` | |
| Eingriff | `intervention` | Source steuerbox/simulation/manuell → `control_box`/`simulation`/`manual` |
| EingriffGrenzeArt je_einrichtung/gesamt | `LimitScope` `per_device`/`total` | |
| § 14a (`Paragraf14a`, `paragraf14a`) | `Paragraph14a` / `paragraph14a` | Foreign name with law citation; models: `Paragraph14aMinuteValue` |
| Paragraf14aModul modul1/modul2 | `Paragraph14aModule` `module1`/`module2` | |
| Netzregeln | `gridRules` | Folder `netzregeln/` → `gridRules/` |
| Netzentgelt (plan decision) | `GridFee` | EnWG term "Netzentgelt", English usually "network charge"; plan keeps `GridFee` |
| Netzentgeltsatz | `GridFeeRate` | |
| Zeitvariables Netzentgelt | `timeVariableGridFee` | |
| Netzleistung | `gridPower` | Positive = import (metering concept) |
| Bezug | `gridImport` | Electricity from the grid |
| Einspeisung (approx. 120) | `feedIn` | Electricity into the grid |
| Einspeisegrenze | `feedInLimit` | E.g. 60 % limit (EEG § 9 para. 2) |
| Einspeisebegrenzung (measure) | `feedInCurtailment` | Pitfall 4 |
| Einspeisevergütung | `FeedInTariff` (as a field `feedInTariff`) | EEG remuneration, ct/kWh |
| Vergütung (general) | `remuneration` | Only outside feed-in |
| Direktvermarktung / feste Vergütung | `directMarketing` / `fixedTariff` | `PvVermarktung` |
| EEG | `eeg` | Foreign name (`eegSection9`) |
| EnWG | `enwg` | |
| Inbetriebnahme (PV under EEG) | `commissioning` | `pvInbetriebnahmeAm` → `pvCommissionedAt`; onboarding with us: `inbetriebnahmeAm` → `onboardedAt`; pitfall 11 |
| Smart Meter (iMSys) | `smartMeter` | |
| Marktlokation / Messlokation | see 2.3 | |
| Netzanschluss | `gridConnection` | |
| Vierzehn-A-Regeln (`vierzehnaRegeln`) | `paragraph14aRules` | The current file name is famously unreadable |

### 2.7 Weather and forecast

| German (as in code) | English | Note |
|---|---|---|
| Wetter | `weather` | Service/packages: `ampiera_wetter` → `ampiera_weather` |
| Wetterdienst | `weatherService` | |
| Prognose | `forecast` | |
| Prognosegüte | `ForecastQuality` | |
| Prognoseablage | `ForecastArchive` | Pitfall 18 (Ablage) |
| Prognosekorrektur | `ForecastCorrection` | |
| PrognoseArt pv/verbrauch | `ForecastKind` `pv`/`consumption` | |
| Verbrauchsprognose | `consumptionForecast` | |
| Strahlung / Einstrahlung | `irradiance` | GHI, DHI etc. stay technical abbreviations |
| Strahlungsaufteilung | `irradianceSplit` | |
| Sonne | `sun` | |
| Schnee | `snow` | `schneeService` → `snowService` |
| Temperatur | `temperature` | |
| Stationen (DWD) | `stations` | |
| Quelle / Quellen | `source` / `sources` | |
| Gitter / Dreiecksgitter | `meshGrid` / `triangularMeshGrid` | **Weather computation mesh**, not the power grid; pitfall 19 |
| Interpolation | `interpolation` | |
| Stundenverteilung | `hourlyDistribution` | |
| Stundenprofil | `hourlyProfile` | |
| Tageswert | `dailyValue` | |
| Tageslage | `dayOverview` | ≠ Anlagenlage |
| Dekodieren | `decode` | `dekodieren/` → `decoding/` |
| Laufplan / Lauf | `runPlan` / `run` | Weather model runs |
| Unsicherheit | `uncertainty` | |
| Höhe | `elevation` | Above sea level, not `height` |
| Feiertage | `publicHolidays` | |
| Geo / Ort / Orte | `geo` / `place` / `places` | Python package `geo/orte` → `geo/places` |
| Gebäude | `building` | `gebaeude.py` → `building.py` |
| Koordinaten | `coordinates` | |
| Speicher (Python package `speicher/`) | `storage` | Persistence, not battery |
| Datenbank | `database` | |
| Eingaben / Schnittstellen (Python) | `inputs` / `interfaces` | |

### 2.8 Money and billing

| German (as in code) | English | Note |
|---|---|---|
| Rechnung | `Invoice` | Files partly `billing.ts` already |
| Rechnungseinstellung | `InvoiceSettings` | |
| Abrechnung | `billing` | |
| AbrechnungStatus | `BillingStatus` | kein_mandat/mandat_aktiv/in_zahlungsverzug/pausiert → `no_mandate`/`mandate_active`/`payment_overdue`/`paused` |
| Mandat (SEPA) | `mandate` | |
| Zahlungsverzug | `paymentOverdue` | |
| AboZahlung | `SubscriptionPayment` | |
| Tarif / Kundentarif | `tariff` / `CustomerTariff` | |
| TarifTyp fest/dynamisch | `TariffType` `fixed`/`dynamic` | |
| Arbeitspreis | `energyPrice` | ct/kWh |
| Grundpreis | `baseFee` | |
| Börsenpreis | `marketPrice` | Already `marketPriceService` in code; day-ahead price |
| Preis / Preise | `price` / `prices` | |
| Kundenpreis | `customerPrice` | |
| Aufschlag | `surcharge` | |
| Betrag | `amount` | |
| Euro / Cent | `eur` / `cent` (`ct`) | Unit suffix: `CtKwh` |
| Netto / Brutto | `net` / `gross` | |
| Umsatz | `revenue` | |
| Kalkulation | `pricingCalculator` | Intranet page; invoicing term `calculation` |
| Provision | `commission` | Partner |
| Vertragsstrafe | `contractualPenalty` | Direction gegen_uns/von_uns → `against_us`/`by_us`; status erfasst/geltend_gemacht/anerkannt/abgelehnt/bezahlt/verfristet → `recorded`/`claimed`/`accepted`/`rejected`/`paid`/`time_barred` |
| GoCardless | `goCardless` | Brand name |
| Steuer | `tax` | |

### 2.9 Staff, permissions, time account

| German (as in code) | English | Note |
|---|---|---|
| Mitarbeiter (`staff`, `AmpieraStaff`) | `Staff` | Already `staff` in code (2300 hits) |
| StaffRolle | `StaffRole` | Block E translates only the type name (values via `@map`); roles are recut in Block R (decision 3) |
| mitarbeiter / support / techniker / admin / ceo / cto / azubi / elektriker | `employee` / `support` / `technician` / `admin` / `ceo` / `cto` / `apprentice` / `electrician` | Values are recut in Block R |
| Abteilung / Stufe | `department` / `level` | Block R: role = department × level |
| Abteilungsleitung | `departmentLead` | |
| Recht / Rechte | `permission` / `permissions` | `hatRecht` → `hasPermission` |
| RollenRecht / StaffRecht | `RolePermission` / `StaffPermission` | |
| Rechte-Editor | `PermissionEditor` | |
| Tresor | `vault` | |
| Geheimnis | `secret` | |
| Passwort | `password` | |
| Anmeldung (login) | `login` | Pitfall 14 |
| Zwei-Faktor | `mfa` | Already `mfaSecretCrypto` |
| Zeitkonto | `TimeAccount` | |
| Zeitbuchung | `TimeEntry` | |
| Zeitpause | `TimeBreak` | |
| ZeitbuchungArt arbeit/urlaub/krank/fehltag/feiertag/urlaub_halb/schule/freizeitausgleich | `work`/`vacation`/`sick`/`absence`/`public_holiday`/`vacation_half`/`vocational_school`/`time_off_in_lieu` | |
| Urlaub | `vacation` | |
| Urlaubsjahr | `VacationYear` | |
| Urlaubsantrag | `VacationRequest` | Kind ganzer_tag/halber_tag/freizeitausgleich → `full_day`/`half_day`/`time_off_in_lieu`; status offen/genehmigt/abgelehnt/zurueckgezogen → `open`/`approved`/`rejected`/`withdrawn` |
| Freizeitausgleich | `timeOffInLieu` | |
| Beschäftigungsart vollzeit/teilzeit/azubi/minijob/werkstudent | `EmploymentKind` `full_time`/`part_time`/`apprentice`/`mini_job`/`working_student` | |
| Arbeitszeitgesetz | `workingTimeAct` | Statute "ArbZG" |
| Feiertag | `publicHoliday` | |
| Azubi | `apprentice` | |
| Berufsschule | `vocationalSchool` | |

### 2.10 Intranet (wiki, tasks, tickets, inbox)

| German (as in code) | English | Note |
|---|---|---|
| Wiki / WikiBuch / WikiKategorie / WikiArtikel | `wiki` / `WikiBook` / `WikiCategory` / `WikiArticle` | |
| Hilfe / Hilfe-Wiki | `help` / `helpWiki` | Files `hilfe/*.md` → `help/*.md` |
| Artikelstatus entwurf / freigegeben / veraltet | `draft` / `published` / `outdated` | `WikiArtikelStatus` (pitfall 17: releasing a text = `published`); "Prüfung fällig" (`pruefung_faellig`, `review_due`) is computed, never stored |
| Prüfung (Wiki-Artikel) / Prüfintervall | `review` / `reviewInterval` | `geprueftAm` → `reviewedAt`, `pruefIntervallMonate` → `reviewIntervalMonths` |
| Rückmeldung (Wiki, „War das hilfreich?“) | `feedback` | `WikiFeedback`, table `wiki_rueckmeldungen`; anonymous, no person column |
| Neuigkeit / Was ist neu | `releaseNote` | `ReleaseNote`, table `release_notes`, files `hilfe/neuigkeiten/*.md`, path `release-notes`; status `draft` / `published` |
| veröffentlichen (Neuigkeit) | `publish` | `publishReleaseNote`, `publishedAt`, `publishedBy`; pitfall 17: releasing a text = `published`; "zurückziehen" = `withdraw` |
| gesehen (Neuigkeiten) | `seen` | `releaseNotesSeenAt` on the staff row, `markReleaseNotesSeen`; "ungelesen" = `unread` (`unreadCount`, `isUnread`), no per-note read table |
| Schnellsuche (Plan 9.8) | `quickSearch` | Search over several sources in the intranet; `POST /api/intranet/v1/search`, `services/search/`; hilfe key `schnellsuche` |
| Treffer (der Suche) | `hit` | `SearchHit`: `id`, `title`, `subtitle`, `matchedIn`, `to` (link path) |
| Quelle (der Suche) | `searchSource` | `SearchSourceKey` wiki/tasks/tickets/partners/receipts/privacyRequests; each is queried only with its right; general "Quelle" stays `source` (2.12) |
| Regel (Vorgabe: Gesetz, Verordnung, Urteil; Block 8d) | `regulation` / `Regulation` | Table `regulations`, number shown as R-n, path `regulations`; NOT the code word "Regel" (`rule`, files `…Rules.ts`); its Stichtag (day it takes effect) is `effectiveOn`, not `cutoffDate`; status `watching` / `relevant` / `implemented` |
| Anwaltsfrage (Block 8d) | `lawyerQuestion` / `LawyerQuestion` | Table `lawyer_questions`, number shown as A-n, path `lawyer-questions`; CONFIDENTIAL (right `anwaltsfragen_verwalten`); status `open` / `asked` / `answered` |
| Tracker-Änderung (Block 8d) | `trackerChange` / `TrackerChange` | Append-only log of regulations and lawyer questions, table `tracker_changes`; text fields are logged by name only |
| betroffene Stelle (Block 8d) | `affectedRef` / `AffectedRef` | Place a regulation or question touches: brain note, code path or board card; JSON list `affected`, at most 20 |
| Checkliste (Eintritt/Austritt, Plan 9.6) | `checklist` / `StaffChecklist` | Tables `staff_checklists`, `staff_checklist_items`, `staff_checklist_log`, template `checklist_template_items`; path `staff-checklists`; NOT the task-board `ChecklistItem` below |
| Eintritt / Austritt (eines Mitarbeiters) | `entry` / `exit` | Kind of a checklist (`ChecklistKind`); in prose `joining` / `leaving` |
| Zuständige(r) (eines Checklistenpunkts) | `assignee` | `assigneeRole`: the role that does the item; the CTO may always do it |
| Deine erste Woche | `firstWeek` | Own entry list, `selfService` items only; `GET /my-first-week`; hilfe key `deine-erste-woche` |
| Selbst erledigen (Checklistenpunkt) | `selfService` | Item the new staff member ticks himself (joining only, no assignee) |
| automatischer Punkt (Checkliste) | `autoKey` / `AutoKey` | Fact of the staff row that ticks the item: `password_changed`, `mfa_working`, `release_notes_seen`, `account_locked` |
| Kalender (Plan 9.7) | `calendar` | Services `services/calendar/`, path `calendar`, tables `calendar_events`, `calendar_event_members`, `calendar_capacity`, `calendar_feed_tokens`, `calendar_log`; hilfe keys `kalender`, `kalender-handy`, `einbau-kalender` |
| Termin | `event` / `CalendarEvent` | `kind`: `meeting` (Besprechung), `internal` (Intern), `private` (Privat), `fitting` (Einbautermin), `customer_appointment` (Kundentermin) |
| Teilnehmer / Zuständige(r) / Besitzer (eines Termins) | `attendee` / `assignee` / `owner` | `MemberRole` of `CalendarEventMember`; business events have only assignees |
| persönlicher Termin / Einbau- und Kundentermin | `personal` / `business` | Two families of kinds: personal has free text and no customer, business has no free text, only customer number, city and plain ids |
| belegt (Termin anderer) | `busy` | `CalendarEventBusy`: only the times; opposite is `full` (`CalendarEventFull`), decided by `eventLevel` |
| automatischer Eintrag (Kalender) | `autoEntry` | Urlaub, Berufsschule, Abwesenheit: computed on read from `Zeitbuchung`, `Schulplan` and the holidays, never stored |
| Kapazität (je Partner / Elektriker und Tag) | `capacity` / `perDay` | `CalendarCapacity`; default 2 per working day; states `open` / `full` / `over` / `closed` |
| Abo-Link / Kalender-Abo (Handy) | `feed` / `feedToken` | ICS feed with the secret token in the path; only the SHA-256 is stored; switch `CALENDAR_FEED_AKTIV` |
| Erinnerung (Kalender) | `reminder` | `reminderMinutes`, `reminderSeenAt` on the member; computed on read for the bell, no stored notification |
| Aufgaben-Board | `TaskBoard` | |
| Aufgabe | `Task` | |
| Spalte (board) | `column` | Prisma model `AufgabenSpalte` → `TaskColumn` |
| Label | `label` | Already English |
| Checklisteneintrag | `ChecklistItem` | |
| Kommentar | `comment` | |
| Mitglied (board) | `member` | eingeladen/angenommen → `invited`/`accepted` |
| Pin | `pin` | |
| Anforderung (aus Konzept, Block 8c) | `conceptRequirement` | Model `ConceptRequirement`, table `concept_requirements`: a note of the concept brain and the card that tracks it; status `open`/`in_progress`/`built`/`rejected`/`card_deleted` derived from the column |
| Systemboard (Block 8c) | `systemBoard` | Board the code creates and finds by `systemKey` (`AufgabenBoard.systemKey`); its columns carry a `statusKey` and cannot be renamed or deleted |
| Ticket / TicketEintrag | `Ticket` / `TicketEntry` | Kind kommentar/status/zuweisung → `comment`/`status`/`assignment` |
| TicketStatus offen/in_arbeit/wartet/erledigt | `open`/`in_progress`/`waiting`/`done` | `erledigt` is `done` everywhere (pitfall 23) |
| Priorität niedrig/normal/hoch/kritisch | `low`/`normal`/`high`/`critical` | |
| TicketKategorie hardware/software/zugang/netzwerk/drucker/sonstiges | `hardware`/`software`/`access`/`network`/`printer`/`other` | |
| Nachricht (model) | `Message` | App/intranet message |
| Benachrichtigung | `notification` | **not** `Nachricht`; pitfall 7 |
| Nachricht gelesen | `MessageRead` | |
| Postfach (plan) / Eingang | `inbox` / `inboxItem` | |
| Notiz / Bemerkung | `note` | |
| Support-Notiz | `SupportNote` | |
| Ereignis (`Ereignis`) | `Event` | `EreignisTyp` info/warnung/fehler/kritisch → `info`/`warning`/`error`/`critical` |
| Störung / Störungsquelle | `fault` / `FaultSource` | Source cloud/geraet_treiber/kunde_verbindung/unbekannt → `cloud`/`device_driver`/`customer_connection`/`unknown` |
| Dashboard / Cockpit | `dashboard` / `cockpit` | |
| Verbindung (Anbindung an ein Fremd- oder Eigensystem, Block 7b) | `connection` | Status view `ConnectionCard`; **not** `Anschluss` (grid connection, `gridConnection`) |
| Kopplung (später: Anbindung einrichten) | `pairing` | Planned for after the status view; not built yet |
| Brain / Konzepte | `brain` / `concepts` | Concept brain keeps the name `brain`, concepts page `concepts` |
| Webseite / Veröffentlichung | `website` / `publication` | `WebseiteInhalt` → `WebsiteContent` |
| Linkseite | `linkPage` | `LinkSeite` einbau/kundenlink → `fitting`/`customer_link` |
| Rechtstext | `LegalText` | Kind `agb` → `terms`, `datenschutz` → `privacy` |
| Papierkorb | `trash` | |
| Einladung | `invitation` | Already `inviteService` → `invitationService` |
| Gelesen | `read` | |
| Anzeige / Darstellung | `display` | Dart: `darstellung_screen` → `appearance_screen` |

### 2.11 Data protection (consent, decommissioning, retention, access log)

| German (as in code) | English | Note |
|---|---|---|
| Einwilligung (approx. 1500) | `Consent` | GDPR Art. 6(1)(a) |
| Einwilligungsverlauf | `ConsentHistory` | |
| Zustimmung (model `Zustimmung`) | `Acceptance` | Acceptance of legal texts, **not** consent; pitfall 20 |
| Zweck | `purpose` | Consent purpose, e.g. `lerndaten` → `learning_data`, `luftbild_analyse` → `aerial_image_analysis`, `mastr_abruf` → `mastr_lookup` |
| Rechtsgrundlage | `legalBasis` | `RechtsgrundlageHemsDaten` → `LegalBasisHemsData`; einwilligung/vertrag → `consent`/`contract` |
| Textfassung | `textVersion` | |
| Widerruf / widerrufen | `withdrawal` / `withdrawn` | Taking back a consent; one word |
| Erteilt / erteilen | `granted` / `grant` | |
| Katalog | `catalog` | |
| Datenschutz | `privacy` | Files/pages; legal area `dataProtection` in comments |
| Datenpanne | `dataBreach` | |
| Löschanfrage | `DeletionRequest` | GDPR Art. 17; status offen/erledigt/abgelehnt → `open`/`done`/`rejected` |
| Löschauftrag (weather service) | `DeletionOrder` | |
| Löschen (permanent) | `delete` | `entferne` → `remove` (from a list) |
| Stilllegung / stillgelegt | `decommissioning` / `decommissioned` | Installation stays as a shell, customer permanently deleted; `STILLLEGUNG_UMFANG` → `DECOMMISSIONING_SCOPE`; pitfall 11 |
| Aufbewahrung | `retention` | `aufbewahrung/` → `retention/` |
| Aufbewahrungsfrist | `retentionPeriod` | |
| Verdichtung | `downsampling` | 1-minute values to 15-minute values; pitfall 21 |
| Zugriff | `access` | |
| Zugriffsprotokoll / EnergiedatenZugriff | `accessLog` / `EnergyDataAccess` | Mandatory log of energy-data access, with reason |
| Zugriffsprotokoll der Mitarbeiter (Plan 9.2) | `staffAccessLog` / `StaffAccessLogEntry` | Append-only log of staff opening customer data, exports, failed logins; not the energy-data log above |
| Zugriffswarnung | `accessWarning` / `AccessWarning` | Warning on conspicuous access; kinds `many_records` / `night_access` / `many_exports` / `failed_logins`; status offen/geprueft → open/checked |
| Energiedaten | `energyData` | |
| Auskunft | `dataAccessRequest` | GDPR Art. 15 |
| Export | `export` | |
| Datenschutz-Anfrage / Vorgang (Plan 9.1) | `privacyRequest` / `PrivacyRequest` | Case for one request under GDPR Art. 15 to 21 with a one-month deadline; table `privacy_requests`, log `PrivacyRequestStep`; number shown as DS-n |
| Art der Anfrage: Auskunft / Berichtigung / Löschung / Widerspruch / Datenübertragbarkeit | `access` / `rectification` / `erasure` / `objection` / `portability` | Kinds of a `PrivacyRequest` (Art. 15 / 16 / 17 / 21 / 20) |
| Eingangsweg | `channel` | `app` / `email` / `letter` (Brief) / `phone` / `other` |
| Identitätsprüfung | `identityVerification` | Methods `app_login` (only the customer's own app request) / `known_email` / `id_document_seen` / `callback_registered_contact` / `other`; access and portability accept only `id_document_seen` and `callback_registered_contact`; only the method is stored, never ID numbers or copies |
| Vier-Augen-Prüfung | `fourEyesReview` | The person who prepared a data release does not review it; `preparedBy` / `reviewedBy` |
| Frist verlängern | `extendDeadline` | GDPR Art. 12(3), up to two more months, reason required |
| Kunde informiert | `customerInformed` | Step `informed_extension` / `informed_result` / `informed_rejection`: a staff member told the customer (Art. 12(3), (4)); required before completing or rejecting |
| Kundenhülle | `customerShell` | Customer state `shell`: reduced by the deletion path (placeholder e-mail, no password, phone or address, all installations decommissioned), kept only for retention duties |
| Löschvorschau | `erasurePreview` | What is deleted and what is kept (with reason and date), computed from the retention rules |
| Ablehnungsgrund | `rejectionReason` | Closed list, see `REJECTION_REASONS` |
| Ablehnung vorbereiten / zurücknehmen | `prepareRejection` / `withdrawRejection` | Steps `rejection_prepared` / `rejection_withdrawn`; the stored reason is what the customer is told before the request is rejected |
| Löschfrist | `deletionDeadline` | |
| Grund | `reason` | Mandatory reason for access |
| Begründung | `justification` | Written justification (permission exception) |
| Stilllegungsumfang | `decommissioningScope` | |
| Mastr-Abruf | `mastrLookup` | |
| Lerndaten-Einwilligung | `learningDataConsent` | |

### 2.12 General words (frequent, always translate the same way)

| German (as in code) | English | Note |
|---|---|---|
| id | `id` | |
| Art | `kind` | `Typ` → `type`; both occur |
| Typ | `type` | |
| Status | `status` | |
| Quelle | `source` | |
| Zeit / Zeitpunkt | `time` / `timestamp` | |
| Zeitraum | `timeRange` | |
| Zeitfenster | `timeWindow` | |
| Datum | `date` | |
| Tag | `day` | |
| Berliner Tag | `berlinDay` | `berlinZeit` → `berlinTime` |
| Beginn / Ende | `start` / `end` | |
| Stichtag | `cutoffDate` | |
| Versatz | `offset` | |
| Dauer | `duration` | |
| Anzahl | `count` | |
| Summe | `total` | `sum` only for a mathematical sum |
| Wert / Werte | `value` / `values` | |
| Feld / Felder | `field` / `fields` | |
| Zeile | `row` | |
| Ergebnis | `result` | |
| Fehler | `error` | `Fehlermeldung` → `errorMessage` |
| Hinweis | `notice` | Pitfall 22 (≠ hint) |
| Meldung | see pitfall 7 | |
| Liste | `list` | |
| Anlass | `trigger` | |
| Grenze / Obergrenze | `limit` / `upperLimit` | |
| Schwelle | `threshold` | |
| Schritt | `step` | |
| Stand / Zustand | `asOf` / `state` | |
| Reihenfolge | `order` | |
| Auswahl | `selection` | |
| Datei / Dateiname | `file` / `fileName` | |
| Ablage | `archive` | Pitfall 18 |
| Schlüssel | `key` | Crypto and lookup key alike; `secret` for confidential values |
| Titel | `title` | |
| Beschreibung | `description` | |
| Inhalt | `content` | |
| Kategorie | `category` | |
| Bezeichnung | `label` | |
| Name | `name` | |
| Version / Fassung | `version` | |
| Modus | `mode` | |
| Nutzer | `user` | |
| Person | `person` | |
| Kalendertag | `calendarDay` | |
| Woche / Monat / Jahr | `week` / `month` / `year` | |
| Stunde / Minute / Sekunde | `hour` / `minute` / `second` | |

---

## 3. Verbs (prefixes in function names)

Derived from approx. 1400 real function names. Number = approximate count of functions with that prefix.

| German | English | Rule / example |
|---|---|---|
| `lies`, `lese` (approx. 390) | `read` | extract from a value you were **handed**: `leseBefehl` → `readCommand`, `leseJson` → `readJson`, `leseZeitpunktMitZone` → `readTimestampWithZone` |
| `lade` (63) | `load` | from our **own** database or file: `ladeAnlage` → `loadInstallation` |
| `hole` (64) | `fetch` or `load` | external system (GitHub, weather, manufacturer) → `fetch`: `holeDependabotZusammenfassung` → `fetchDependabotSummary`; database → `load`: `holeNetzentgeltFuerAnlage` → `loadGridFeeForInstallation` |
| (no prefix, pure access) | `get` | in-memory only, no I/O (getter, map lookup). Database → `load…`, lookup that may return `null` → `find…`; the rule-book example is `findCustomerById` |
| `suche` | `search` | `sucheOrt` → `searchCity`; returns a list |
| `finde` | `find` | Result may be `null` |
| `liste` | `list` | `listeEingaenge` → `listInboxItems` |
| `plane` (36) | `plan` | `planeLadebedarf` → `planChargingNeed`; rule book: `planSchedule` |
| `rechne` (23) | `calculate` | pure calculation logic: `rechneFahrplan` → `calculateSchedule` |
| `berechne` (12) | `calculate` | same meaning as `rechne` |
| `bestimme` | `determine` | fix/derive a value: `bestimmeLage` → `determineGeoPosition` |
| `ermittle` | `detect` or `determine` | faults/anomalies → `detect`: `ermittleProbleme` → `detectProblems` |
| `bewerte` (18) | `evaluate` | `bewertePvTag` → `evaluatePvDay` |
| `zaehle` (14) | `count` | `zaehleAbdeckung` → `countCoverage` |
| `pruefe` – rule (117 `pruefe…` in total) | `check` | business check of a state: `pruefeZeitfenster` → `checkTimeWindow`, `pruefeEingriff` → `checkIntervention`, `pruefeMinderertrag` → `checkUnderperformance` |
| `pruefe` – input | `validate` | form/type of input: `pruefeTextEingabe` → `validateTextInput`, `pruefeAbfrage` → `validateQuery`, `pruefeLaenge` → `validateLength`, `pruefeUrl` → `validateUrl` |
| `pruefe` – authenticity | `verify` | token, code, signature: `pruefeTotpCode` → `verifyTotpCode`, `pruefeLoginCode` → `verifyLoginCode`, `pruefeVorschauToken` → `verifyPreviewToken`, `pruefeSimToken` → `verifySimToken` |
| `pruefen`, `geprueft`, `Pruefung` | `check` / `checked` / `check` (noun) | noun `Pruefung` → `validation` only for input, otherwise `check` |
| `baue` (33) | `build` | assemble without calculation: `baueEinbauHtml` → `buildFittingHtml`, `baueAntwort` → `buildResponse` |
| `erzeuge` – factory/new | `create` | `erzeugeEreignis` → `createEvent`, `erzeugeGoeSender` → `createGoeSender`, `erzeugeApi` → `createApi` |
| `erzeuge` – artefact | `generate` | `erzeugeCode` → `generateCode`, `erzeugeOnboardingPdfBuffer` → `generateOnboardingPdfBuffer` |
| `lege…an`, `anlegen` | `create` | `legeNetzentgeltsatzAn` → `createGridFeeRate` |
| `lege…ab` | `store` | `legeAb` → `store` (Ablage → archive) |
| `speichere`, `speichern` (72) | `save` | business save to the database: `speichereFelder` → `saveFields` |
| `schreibe` (21) | `write` | technical write to file/stream: `schreibeDatei` → `writeFile` |
| `setze` (35) | `set` | `setzeModus` → `setMode` |
| `loesche` (16) | `delete` | permanent: `loescheEingang` → `deleteInboxItem` |
| `entferne` | `remove` | from a list/structure |
| `sende` (25) | `send` | `sendeLoginCode` → `sendLoginCode` |
| `melde` (24) | `report` | report an event/state: `meldeEreignis` → `reportEvent` |
| `registriere` (16) | `register` | routes/handlers |
| `starte` / `stoppe` (24/22) | `start` / `stop` | `starteSofortladung` → `startInstantCharge` |
| `beende` (11) | `end` or `finish` | session/charging session → `end`; procedure/flow → `finish` (`beendeOAuth` → `finishOAuth`) |
| `fuehre … aus` (12) | `execute` | `fuehreLaufAus` → `executeRun` |
| `waehle` (25) | `select` | `waehleReferenztag` → `selectReferenceDay` |
| `ordne` (17) | `map` | `ordneLgGeraete` → `mapLgDevices` (map manufacturer data onto our device) |
| `normalisiere` | `normalize` | |
| `gleiche … ab` | `reconcile` | `gleicheStandortAb` → `reconcileLocation` |
| `sperre` (verb) | `lock` | `sperreLink` → `lockLink` |
| `oeffne` / `schliesse` | `open` / `close` | |
| `warte` | `wait` | `warteAufVerbindung` → `waitForConnection` |
| `als…` (45) | `to…` / `as…` | conversion: `alsListe` → `toList`, `alsDatum` → `toDate`; `AlsJson` → `ToJson` |
| `ist…` (123) | `is…` | `istSimulation` → `isSimulation` |
| `hat…` (19) | `has…` | `hatRecht` → `hasPermission` |
| `darf…` / `kann…` | `can…` | `darfVerarbeiten` → `canProcess`, `kannBegrenzen` → `canLimit` |
| `mit…` / `ohne…` | `with…` / `without…` | |
| `…fuer…` | `…For…` | `holeNetzentgeltFuerAnlage` → `…ForInstallation` |
| `format…` | `format…` | `formatDauer` → `formatDuration`, already English |
| `parse…` / `require…` / `handle…` / `on…` / `dispose` | unchanged | already English (Dart/React patterns) |

---

## 4. Pitfalls

1. **Messlokation, Zählpunkt, Messstelle.** Internationally "metering point" is the **Zählpunkt** (= Messlokation). The plan assigns `MeteringPoint` to the **Messstelle** (MsbG). Resolution: Messstelle = `MeteringPoint`, Messlokation (= Zählpunkt) = `MeteringLocation`, never `MeteringPoint` for the ID. The doc comment names both German terms. Marktlokation = `MarketLocation`, not the same as Messlokation (schema comment).
2. **Zählerwert ≠ Zählerstand.** `Zaehlerwert` in code is the **energy per quarter hour** → `MeterValue`. `meterReading` is the cumulative reading and must not be used for it.
3. **Netzbetreiber ≠ Messstellenbetreiber.** `GridOperator` (distribution grid) and `MeteringPointOperator` (us) must never both be called just "operator". Not `NetworkOperator`, not `Dso`.
4. **Abregelung ≠ Drosselung ≠ Einspeisebegrenzung.** Abregelung = PV feed-in cut (profitability, feed-in limit) → `curtailment`. Drosselung = § 14a power reduction of a consumer → `throttling`. Einspeisebegrenzung = the measure/function → `feedInCurtailment`; Einspeisegrenze = the value → `feedInLimit`. Never "Drosselung" for PV and never `limit` for the verb (`limit` is the schedule state `begrenzen`).
5. **Lage has three meanings.** `lageAusAntwort`, `Anlagenlage` = geo position (`geoPosition`); `Tageslage` = daily overview (`dayOverview`); "Lage" in the sense of situation does not exist in the code. Not `situation`, not `position` alone.
6. **Wirkung ≠ Wirkungsgrad.** `WirkungTag`/`Wirkungsnachweis` = benefit for the customer (`impact`); `Wirkungsgrad` = `efficiency`. Both start with "Wirkung" and get mixed up.
7. **Meldung, Nachricht, Benachrichtigung.** Nachricht = `Message` (model), Benachrichtigung = `notification` (push/banner), Meldung is ambiguous: Steuerbox-Meldung → `controlBoxMessage`, watchdog report → `alert`, partner report → `partnerReport`, Fehlermeldung → `errorMessage`. The verb `melde…` is `report`.
8. **Anschlussnehmer ≠ Kunde.** The Anschlussnehmer owns the grid connection (can be a third party, `AnschlussnehmerArt kunde|dritter`) and is not automatically the contract partner. Not `customer`, not `connectionCustomer`.
9. **Speicher has two meanings.** Battery storage / `speicher*` in the backend = `battery`; `speicher/` in the weather service and "Speicher" as persistence = `storage`. And `speichere` (verb) = `save`. Never `storage` for the battery, never `battery` for the database.
10. **Zugang.** `GeraeteZugang` = the customer's account at the manufacturer (token) = `DeviceAccount`; not `DeviceAccess`. Zugriff (data protection) = `access`, hence `accessLog`.
11. **Stilllegung ≠ Inbetriebnahme ≠ Betrieb.** `stillgelegtAm` = installation frozen after customer deletion → `decommissionedAt`; `inbetriebnahmeAm` (onboarding with us) and `pvInbetriebnahmeAm` (EEG commissioning of the PV) would both become `commissionedAt`, **a conflict in the model `Anlage`**: hence `onboardedAt` (with us) and `pvCommissionedAt` (EEG). `NUR_BETRIEBENE_ANLAGEN` → `ONLY_OPERATED_INSTALLATIONS`.
12. **Protokoll has two meanings.** `Befehlsprotokoll`, `Zugriffsprotokoll`, event log = log → `log`; `Einbauprotokoll`, Messprotokoll = document → `record`.
13. **Takt, Taktung.** `Takt` (scheduler cycle, `ausfuehrungsTakt`) = `tick`; `Taktung` (heat pump/battery switches too often) = `shortCycling`. Files `takt.ts` exist in three folders.
14. **Anmeldung.** Login in app/intranet = `login`; Anmeldung at the grid operator (market process) = `registration`; `anmeldungSeite` in the backend (customer-link page under `/anmeldung`) = `signupPage`; `einbauSeite` = `fittingPage`. Three different things.
15. **Bestätigung ≠ Quittung.** `bestaetigt` = the device carried out the command → `confirmed`; `Quittung` = ACK/NACK of the control box (VDE FNN) → `acknowledgement`. `bestaetigungsWaechter` → `confirmationWatchdog`.
16. **Sofortladung ≠ Sofortneuplanung.** `Sofortladung` = customer presses "charge now" → `InstantCharge`; `sofortNeuplanung` = the schedule is recalculated immediately → `immediateReplanning`. Do not replace the prefix "sofort" wholesale.
17. **Freigabe has three meanings.** Approval by a person → `approval`; grid charging released (rule allows it) → `allowed`; publication (website, legal text) → `published`. Read the context when renaming.
18. **Ablage.** `Prognoseablage`, `legeAb` = buffer/archive → `archive`/`store`; `Ablage` in plan block 4 (central file directory) → `fileRegistry`. Not `storage`.
19. **Netz and Gitter.** `netz*` = power grid (`grid`); `Gitter/Dreiecksgitter` in the weather service = computation mesh of the weather models. Hence `grid` only for the power grid, `meshGrid` in the weather service.
20. **Zustimmung ≠ Einwilligung.** `Einwilligung` (data-protection purpose) = `Consent`; `Zustimmung` (acceptance of terms/legal texts) = `Acceptance`; `EinbauZustimmung` (owner permits the fitting) = `FittingApproval`. Not everything is `Consent`.
21. **Verdichtung.** Not `compression`; it means merging fine measurements into coarser ones = `downsampling`. The file `aggregationService` is already English and means something else (daily aggregation → `dailyAggregation`).
22. **Hinweis ≠ Tipp.** The code means a business note to the user (schedule note), `notice`; `hint` is for tiny UI texts (`aria`, placeholders).
23. **Erledigt.** `done` everywhere (task, ticket, onboarding, deletion request); `resolved` and `completed` are out. But `abgeschlossen` (reference day) = `completed`, and `geliefert` = `delivered`.
24. **Manufacturer names** (`viessmann`, `myuplink`, `daikin_onecta`, `sma`, `goe_cloud`, `enode`, `shelly` …) are brand names and stay unchanged.

---

## 5. API paths

Rule: `/api/<area>/v1/<path>`. The six areas keep their names (approved): `app` (customer app, customer link, fitting link), `intranet` (staff, VPN only), `web` (public website), `dienst` (server to server), `integration` (third-party systems), `kern` (shared core).

Path segments are kebab-case English words **derived from the glossary term**, not translated freshly:

- Model/type name in the plural for collections: `MeteringPoint` → `metering-points`, `RoofSurface` → `roof-surfaces`, `FittingRecord` → `fitting-records`, `Installation` → `installations`.
- Compound names are split at the camelCase boundary: `evCharger` → `ev-charger`, `heatPump` → `heat-pump`, `immersionHeater` → `immersion-heater`, `Paragraph14a` → `paragraph-14a`, `ControlMode` + report → `control-mode-report`.
- Singular for one resource of a parent and for actions: `/devices/:deviceId/specs/battery`, `/installations/:installationId/fitting-check`.
- Path parameters are camelCase with the entity as prefix: `:installationId`, `:deviceId`, `:customerId`.
- No umlaut transcription, no abbreviations (`msb`, `gwa`), no German words. User-facing texts are not part of paths.
- A new `/api` route needs an entry in `src/routes/areas/routeMap.ts` (the guard test `routeAreas.test.ts` fails otherwise). The old German paths under `/api/v1/…` stay as aliases until they are removed; new code uses the area paths only.

Examples:

| Glossary term | Path segment |
|---|---|
| `Installation` | `/installations/:installationId` |
| `specs` + `evCharger` | `/devices/:deviceId/specs/ev-charger` |
| `FittingRecord` | `/installations/:installationId/fitting-records` |
| `FittingCheck` | `/installations/:installationId/fitting-check` |
| `MeteringPoint` | `/installations/:installationId/metering-points` |
| `RoofSurface` | `/installations/:installationId/roof-surfaces` |


Path terms without their own glossary row (Block 7a, 06.10.2026):

| German (old path) | Path segment | Note |
|---|---|---|
| Vorgang (Fristen) | `cases` | one market process case; model `MarketProcess` |
| Zeitbuchungsantrag | `entry-requests` | request for a `TimeEntry` |
| Monatsabrechnung (Zeitkonto) | `month-closings` | closing of a time-account month, not invoicing (`billing`) |
| Glocke | `bell` | counter for the header bell |
| Stammdaten (Mitarbeiter) | `master-data` | |
| Berufsschulplan | `school-plan` | `vocationalSchool` |
| Strafen (Fristen) | `penalties` | `contractualPenalty` |
| neu suchen | `rediscover` | find devices again at the manufacturer |
| Lage neu bestimmen | `geo-position/redetermine` | |
| MSB-Verbrauch | `metering-consumption` | `consumption` read as metering point operator |
| Einträge (IT) | `entries` | IP entries of a VLAN |
| Hersteller-Zugang / Cloud-Zugang / Token-Zugang | `device-accounts` / `cloud-account` / `token-account` | pitfall 10: `account`, never `access` |
| Netzladen-Freigabe | `grid-charging-allowance` | pitfall 17: a rule allows it, no person approves |
| Anlagenwächter-Meldungen | `installation-watchdog/alerts` | pitfall 7: watchdog message = `alert` |
| Eichfristen | `calibration-periods` | `calibrationPeriod` |

---

## 6. Decisions (05.10.2026)

Noel (CTO) accepted all recommendations:

1. **Legal terms:** established EDI@Energy/EU terms in English (`MeteringLocation`, `MarketLocation`, `GatewayAdministrator`, `MeteringPointOperator`, `GridOperator`, `FeedInTariff`); names of laws and registers stay foreign names (`paragraph14a`, `wim`, `mastr`, `obis`, `eeg`); abbreviations (MaLo, MSB, GWA) only in comments.
2. **Wallbox = `evCharger`** (device in the home); `chargePoint` only in the OCPP part (`OcppChargePoint`); user text keeps "Wallbox".
3. **Roles:** Block E translates only the type name (`StaffRole`, values via `@map`); role values are recut in Block R.
4. **Enums:** type names in Block E; enum values with Block 7a's new API; old paths keep the old values via the `…AlsJson` converters.
5. **Verb `get`** only for pure in-memory access; database → `load…`, possibly-null lookup → `find…`; the rule-book example becomes `findCustomerById`.
6. **Installation and fitting:** Anlage = `Installation`, Einbau (the activity) = `Fitting` (draft recommendation, not answered explicitly).
7. **API areas** `app`, `intranet`, `web`, `dienst`, `integration`, `kern` keep their names.
