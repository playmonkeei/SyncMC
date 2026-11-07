# Zu SyncMC beitragen

SyncMC ist ein Minecraft-Java-Launcher in früher Entwicklung. Hilfreich sind reproduzierbare Fehlerberichte, sorgfältige Tests auf weiteren Betriebssystemen und überschaubare Verbesserungen an Oberfläche, Installation und Profilverwaltung.

## Lokal loslegen

Die [Entwicklungsanleitung](docs/development.md) beschreibt Voraussetzungen, Browser-Vorschau, Desktop-App und Tests. Der aktuelle Funktionsumfang und offene Freigaben stehen in der [README](README.md).

```sh
npm ci
npm run dev
```

Für native Funktionen nach Einrichtung der Tauri-Voraussetzungen `npm run desktop` verwenden. Dieser Befehl startet den Entwicklungsserver selbst; einen bereits laufenden Server auf Port 4200 vorher beenden.

## Fehler melden

Suche zunächst nach einem bestehenden Issue. Ein neuer Bericht sollte enthalten:

- Betriebssystem, Architektur und verwendeten Commit oder Versionsstand.
- Kurze Schritte zum Nachstellen sowie erwartetes und tatsächliches Verhalten.
- Bei Installationsproblemen die Minecraft-Version, den Loader und die Inhaltsquelle.
- Einen passenden Screenshot oder einen gekürzten, bereinigten Logauszug.

Entferne vor dem Hochladen Zugangstokens, Benutzer-Codes, API-Schlüssel, persönliche Kontodaten und private Dateipfade. Die öffentliche Microsoft-Client-ID ist eine Anwendungskennung und kein Geheimnis. Der aktuelle Freigabe- und Prüfstand ist im [Anmeldestatus](docs/authentication.md) dokumentiert.

## Änderungen vorschlagen

Beschreibe bei größeren Änderungen zuerst in einem Issue das Problem und das gewünschte Verhalten. Für eine kleine Korrektur genügt ein klar beschriebener Pull Request.

1. Einen eigenen Branch anlegen und die Änderung auf ein zusammenhängendes Thema begrenzen.
2. Vorhandene Muster in Angular, TypeScript und Rust beibehalten.
3. Betroffene Abläufe prüfen und bei Änderungen am Verhalten passende Tests ergänzen.
4. Geänderte Einrichtungsschritte, Profilformate oder Einschränkungen dokumentieren.
5. Im Pull Request erklären, was sich für Nutzer ändert und welche Prüfungen ausgeführt wurden. Bei sichtbaren Änderungen einen Screenshot ergänzen.

Die regulären Prüfungen sind:

```sh
npm test
npm run build
npm run test:native
```

Für reine Dokumentationsänderungen genügen die Prüfung der Aussagen, Links und Darstellung. Native Änderungen zusätzlich mit der Desktop-App prüfen; Browser-Tests simulieren die Tauri-Schnittstelle und ersetzen diese Prüfung nicht. Große Netzwerktests bleiben ausdrücklich optional; Details stehen in der [Entwicklungsanleitung](docs/development.md#optionale-netzwerktests).

Vor dem Commit die gewählten Dateien mit `git add` stagen und `npm run check:publication` ausführen. Mit `npm run hooks:install` lässt sich dieselbe Prüfung als lokaler Pre-Commit-Hook aktivieren. Das benötigt Node.js auch beim Committen. Bilder und Logs zusätzlich selbst durchsehen; Details stehen in [SECURITY.md](SECURITY.md).

Beiträge werden unter der Projektlizenz [GPL-3.0-only](LICENSE) eingebracht. Drittanbieterdateien und ihre Lizenzhinweise müssen entsprechend gekennzeichnet bleiben.

## Abhängigkeiten aktualisieren

Dependabot prüft npm und Cargo wöchentlich und aktualisiert die Lockdateien innerhalb der bereits deklarierten Versionsbereiche. Bei regulären Versionsupdates werden Angular und TypeScript sowie die gemeinsam verwendeten RustCrypto-Pakete jeweils zusammengefasst. GitHub-Actions-Updates bilden eine eigene Gruppe. Die Regeln stehen in [.github/dependabot.yml](.github/dependabot.yml); die Bedeutung von `lockfile-only` erklärt die [offizielle Referenz](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference#versioning-strategy).

Neue Versionsbereiche werden bewusst migriert: Peer-Abhängigkeiten und API-Änderungen prüfen, die Manifest- und Lockdateien gemeinsam ändern und die vollständige CI abwarten. Bei Cargo können auch Versionswechsel vor 1.0 inkompatibel sein. Ein isoliertes Update eines von mehreren gemeinsam verwendeten `Digest`-Paketen genügt beispielsweise nicht.

Sicherheitswarnungen bleiben aktiv. `lockfile-only` kann auch automatische Sicherheits-PRs begrenzen, wenn die Korrektur eine Manifeständerung benötigt. Solche Warnungen müssen deshalb geprüft und die nötigen Versionswechsel als eigene Migration bearbeitet werden. Ein erfolgreicher Testlauf allein ersetzt keine Prüfung der Änderungen bei Archivverarbeitung, Prüfsummen oder Kontodaten.

## Leitlinien für die Implementierung

- Gemeinsame Instanzprofile und lokale Gerätewerte getrennt halten. Bestehende Profile bei Formatänderungen berücksichtigen.
- Keine Passwörter, Tokens oder Provider-Schlüssel in Repository, Weboberfläche, Exporten oder Logs aufnehmen. Native Zugangsdaten gehören in den Betriebssystem-Kontospeicher.
- Originaldateien und Moddownloads mit den vorgesehenen Prüfsummen validieren. Anbieterfreigaben und Downloadbeschränkungen berücksichtigen.
- Bei Dateiänderungen Sicherungs-, Reparatur- und Abbruchpfade erhalten. Ein fehlgeschlagener Download darf nicht als fertige Installation erscheinen.
- Einschränkungen offen benennen. Simulierte Abläufe, geprüfte Installation und authentifizierter Spielstart sind unterschiedliche Prüfergebnisse.

Bitte respektvoll und konkret diskutieren. Kritik sollte das Verhalten oder den Code beschreiben und einen nachvollziehbaren Verbesserungsvorschlag enthalten.
