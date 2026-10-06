# Contributing

The Ampiera main rule set (`~/Documents/Github/CLAUDE.md`, "Ampiera-Hauptregelwerk") applies; this file and
`CLAUDE.md` in this repo add what is specific here.

## Names and language

- Code, identifiers, comments and docs in English; German terms follow `docs/GLOSSARY.md` (same list as in
  all Ampiera repos). Wallbox in code is `ev_charger` / `evCharger`; `charge_point` only in the OCPP part.
- Text a person sees (UI, error messages, scenario checks) is German with real umlauts, informal "du".
  Use "Wallbox" (never "Box"), "Zentrale" for the OCPP counterpart, "Produktivserver" for the live instance,
  "App-Sicht" for the app view.
- Rust: snake_case functions, PascalCase types, UPPER_SNAKE constants. TypeScript: camelCase, components
  PascalCase.

## Rules that are easy to break

- Missing measurement is `None`/`null`, never `0`.
- Assumptions are named constants with a reason.
- Pure logic stays free of IO and gets tests with a counter-check.
- Safety rules live in the core (`policy.rs`, `gate.rs`), never only in the UI.
- No secrets in files, logs, frames, events, errors or exports.
- No colour, spacing or font value outside `src/styles/tokens.css`; no inline `style` attributes (CSP).
- Changing a type, command or event: change `docs/CONTRACT.md` first, then Rust, shell and UI together.
- Changing what the simulator expects from the server: update `docs/CENTRAL_SYSTEM_BEHAVIOUR.md` with the
  backend commit you read it from.

## Before every commit

```
cd ~/Documents/Github/ampiera-device-simulator && npm run lint && npx prettier --check src && npm test && npm run build
cd ~/Documents/Github/ampiera-device-simulator && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Stage files explicitly (no `git add -A`). Commit author: `Noel Frömbgen <n.froembgen@ampiera.de>`.
