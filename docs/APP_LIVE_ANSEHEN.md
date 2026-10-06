# App-Sicht und echte App live ansehen

Die App-Sicht im Simulator zeigt, was die Kunden-App vom Server zu einer simulierten Wallbox bekommt. Sie
liest nur, höchstens alle 30 Sekunden. Daneben kannst du dieselben Daten in der echten App auf dem iPhone
anschauen.

## 1. Testkunde vorbereiten (einmalig, auf dem Server)

Der Testkunde des Hersteller-Simulators (`herstellersim@ampiera.invalid`) hat noch kein Passwort. Eine
Einladung stellst du so aus:

```
cd /opt/ampiera && docker compose exec backend node dist/scripts/herstellersim.js --app-einladung
```

Die Ausgabe zeigt E-Mail, Einladungscode und Ablaufdatum (7 Tage) genau einmal. Nicht in den Chat und nicht
in Dateien kopieren.

## 2. Einladung im Simulator einlösen

App-Sicht → „Einladung einlösen“ → E-Mail, Einladungscode, neues Passwort (mindestens 15 Zeichen, darf in
keinem bekannten Datenleck vorkommen). Danach mit E-Mail und Passwort anmelden.

**Wichtig für den Produktivserver:** Ist dort ein Mailserver eingerichtet, verlangt die Anmeldung von einem
neuen Gerät einen 6-stelligen Code per E-Mail. Der geht an `herstellersim@ampiera.invalid` und kommt nie an.
Gegen den Produktivserver brauchst du dann einen Testkunden mit echtem Postfach (offene Entscheidung). Lokal
ohne Mailserver gibt es diese Prüfung nicht.

## 3. Echte App daneben

In der TestFlight-App mit derselben E-Mail und demselben Passwort anmelden. Auch hier gilt die
Geräteprüfung aus Schritt 2. Die App zeigt dieselben Daten wie die App-Sicht.

## Was du heute sehen wirst

- Verbindung „online“, sobald Messwerte der Wallbox ankommen.
- Viertelstunden-Energie unter „Letzte Viertelstunde“, etwa 3 Minuten nach Ende der Viertelstunde.
- **Keine** aktuelle Ladeleistung und Status „offline“ für eine reine OCPP-Wallbox: Das ist eine bekannte
  Lücke im Backend (`docs/CENTRAL_SYSTEM_BEHAVIOUR.md`, letzter Abschnitt), kein Fehler des Simulators.
  Szenario S11 meldet sie.
