# Sicherheit und vertrauliche Hinweise

SyncMC ist eine frühe Alpha. Sicherheitskorrekturen werden für den aktuellen Stand auf `main` vorgenommen; ältere Entwicklungsstände erhalten keine zugesicherten Updates.

## GLib-Sicherheitsfix für Linux

Tauris GTK3-Abhängigkeiten verwenden GLib 0.18. SyncMC bindet dafür eine lokale
GLib-0.18.5-Kopie mit dem offiziellen Fix für
[RUSTSEC-2024-0429 / GHSA-wrw7-89jp-8q8g](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)
ein. Der Fix korrigiert den veränderlichen Ausgabezeiger in `VariantStrIter`.
Die ursprüngliche Versionsnummer und MIT-Lizenz bleiben erhalten.

[Herkunft, Archivprüfsumme und Upstream-Commit](src-tauri/vendor/README.md) sind
dokumentiert. CI prüft die vollständige Quellintegrität und führt die betroffenen
Iteratoroperationen unter Linux mit Compileroptimierungen aus. Der Backport kann
entfallen, sobald Tauri durchgängig GLib 0.20 oder neuer unterstützt.

## Eine Sicherheitslücke melden

Bitte veröffentliche keine Zugangsdaten, Kontoinformationen oder ausnutzbaren Details in einem öffentlichen Issue. Wenn im Repository **Security → Advisories → Report a vulnerability** verfügbar ist, verwende die private Meldung. Andernfalls eröffne ein Issue mit einer allgemeinen Bitte um einen vertraulichen Meldeweg und warte mit technischen Details, bis dieser vereinbart ist.

Für eine private Meldung helfen die betroffene Version, Betriebssystem, erwartetes und tatsächliches Verhalten sowie möglichst kleine Schritte mit Testdaten. Eine feste Antwortfrist wird derzeit nicht zugesichert.

## Logs und Screenshots teilen

- Keine Passwörter, API-Keys, Access-/Refresh-Tokens, Cookies oder aktiven Anmeldecodes teilen.
- Profilnamen, UUIDs, E-Mail-Adressen, private Serveradressen und persönliche Dateipfade entfernen.
- Konfigurationen und Profil-Exporte vor dem Hochladen prüfen. Serverfavoriten können private Adressen enthalten.
- Für Screenshots ein frisches Profil ohne angemeldetes Konto verwenden.

## Vor einem Commit

Nach `git add` prüft `npm run check:publication` die tatsächlich gestagten Dateinamen und Textinhalte auf bekannte Credential-Muster, persönliche Home-Pfade und private Dateitypen. Die Prüfung gibt nur Datei, Zeile und Fundtyp aus. Sie überträgt keine Dateien an einen Dienst und ersetzt keine manuelle Durchsicht; insbesondere Bildinhalte müssen separat geprüft werden.

Einzelne geprüfte synthetische Testzeilen sind ausschließlich über Dateipfad, SHA-256 der vollständigen Zeile und den jeweiligen Fundtyp ausgenommen; Änderungen an diesen Zeilen erfordern eine erneute Prüfung.

Die `.gitignore` schließt lokale Konfiguration, Schlüsseldateien, App-Daten, interne QA-Belege und Build-Ausgaben aus. Eine Ignore-Regel entfernt bereits versionierte Dateien nicht automatisch. Deshalb immer zusätzlich `git diff --cached --stat` und `git diff --cached` prüfen.

Die Microsoft-Public-Client-ID in `app-config.ts` ist eine bewusst öffentlich ausgelieferte Anwendungskennung. Client Secrets und persönliche Zugangsdaten gehören niemals in das Repository.
