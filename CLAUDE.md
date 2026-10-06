# Ampiera-Regelwerk – ampiera-device-simulator

> Gilt **zusätzlich** zum Ampiera-Hauptregelwerk (`~/Documents/Github/CLAUDE.md`). Stand: 06.10.2026.

## Was das Repo ist

Desktop-App (Tauri 2, Mac und Windows), die OCPP-1.6J-Wallboxen gegen die Ampiera-Zentrale simuliert,
Prüfszenarien S1–S11 fährt und in der App-Sicht zeigt, was die Kunden-App bekommt. Liegt vorerst auf Noels
persönlichem GitHub-Account (`Noel-Faltrix/ampiera-device-simulator`, privat); bei Übernahme per „Transfer
ownership“ in die Organisation `AmpieraSolutions`.

## Feste Regeln hier

- **Vertrag zuerst:** Typen, Commands, Events stehen in `docs/CONTRACT.md`. Änderung dort, dann Rust-Kern,
  Hülle und UI gemeinsam.
- **Server-Verhalten:** Was der Simulator von der Zentrale erwartet, steht in
  `docs/CENTRAL_SYSTEM_BEHAVIOUR.md` mit dem Backend-Commit, aus dem es gelesen ist. Ändert sich das Backend,
  zuerst dort nachziehen.
- **Schutz des Produktivservers liegt im Kern** (`policy.rs`, `gate.rs`), nie nur in der UI: Live wird aus
  dem Host abgeleitet, höchstens 3 Live-Wallboxen online, S9/S10 nie gegen Live, 60 s Sperre nach 401/429.
- **Keine Geheimnisse** in Dateien, Logs, Frames, Events, Fehlertexten oder Exporten. OCPP-Passwörter nur
  im Schlüsselbund, App-Tokens nur im Speicher.
- **Kein App-Repo anfassen:** Die App-Sicht liest nur die Kunden-API; keine Änderungen in `ampiera-app`,
  kein Nachbau von App-Bildschirmen.
- **Texte:** deutsch, echte Umlaute, „du“, „Wallbox“ (nie „Box“), „Zentrale“, „Produktivserver“,
  „App-Sicht“.
- **Design:** nur Tokens aus `src/styles/tokens.css`, keine Inline-Styles (CSP).

## Prüfen

Siehe `CONTRIBUTING.md`. Im Cloud-Arbeitsbereich laufen alle Prüfungen inkl. `cargo test --workspace`;
macOS- und Windows-Installer baut nur GitHub Actions oder Noel lokal. `npm install` nie über die Bridge.

## Brain

Notiz `08.xx Geräte-Simulator (Desktop) für die OCPP-Zentrale` in `ampiera-brain`; bei Abschluss eines
Pakets anfassen.
