# Entwicklung und Tests

[Zur README](../README.md) · [Beiträge](../CONTRIBUTING.md) · [Anmeldung](authentication.md) · [Inhaltsquellen](content-sources.md)

SyncMC besteht aus einem Angular-Frontend und einem eigenen Rust-Kern mit Tauri. Die Browser-Vorschau eignet sich für Oberfläche, Instanzverwaltung und Modrinth-Suche; Minecraft-Installation, Java-Verwaltung, Kontospeicher und Spielstart benötigen die Desktop-App.

## Voraussetzungen

- **Node.js 24.16.0 und npm.** Die Projektversion steht in [.nvmrc](../.nvmrc); die Angular-Abhängigkeiten legen die unterstützten Versionen über ihre `engines` fest.
- **Rustup** für native Entwicklung. Die projektgebundene Toolchain steht in [rust-toolchain.toml](../rust-toolchain.toml) und wird beim Aufruf von Cargo im Projekt verwendet. Die globale Standardversion muss nicht geändert werden.
- **Plattformwerkzeuge für Tauri 2** für native Builds und Rust-Tests. Dazu gehören unter macOS die Xcode Command Line Tools, unter Windows die C++-Buildwerkzeuge und WebView2 sowie unter Linux die passenden Systembibliotheken. Die verbindlichen Installationsschritte stehen in den [offiziellen Tauri-Voraussetzungen](https://v2.tauri.app/start/prerequisites/).

Java ist keine Voraussetzung zum Bauen der Oberfläche. Für einen Spielstart benötigt SyncMC eine zur Minecraft-Version passende Laufzeit; die Desktop-App kann diese lokal einrichten. Der optionale Fabric-Integrationstest erwartet bereits ein auffindbares Java 21.

## Installation und Start

Alle Befehle im Projektverzeichnis ausführen:

```sh
npm ci
```

`npm ci` installiert den Stand aus `package-lock.json`. Bei beabsichtigten Abhängigkeitsänderungen die zugehörige Lockdatei mit aktualisieren; für Rust gilt entsprechend `src-tauri/Cargo.lock`.

Für die Browser-Vorschau:

```sh
npm run dev
```

Danach [127.0.0.1:4200](http://127.0.0.1:4200) öffnen. Die Vorschau verwendet den lokalen Angular-Proxy für Modrinth und Mojangs Versionsmanifest. CurseForge-Anfragen mit Schlüssel bleiben im nativen Kern.

Für die Desktop-App:

```sh
npm run desktop
```

Tauri startet den Angular-Server selbst. Einen separat laufenden Server auf Port 4200 vorher beenden. Die App speichert ihre Daten im betriebssystemspezifischen Verzeichnis für `dev.syncmc.desktop`; auf macOS ist dies `~/Library/Application Support/dev.syncmc.desktop/`. Instanzdaten liegen dort unter `minecraft/`. Entwicklungsstarts verwenden diesen lokalen App-Datenbestand, daher für Tests mit bestehenden Welten vorher eine Sicherung anlegen.

## Builds

```sh
npm run build
npm run desktop:build
```

Das Web-Build liegt unter `dist/syncmc/browser/`. Es enthält keinen öffentlichen API-Proxy; eine statisch gehostete Kopie ersetzt den lokalen Entwicklungsserver und dessen Kataloganbindung nicht.

Der Desktop-Build kompiliert das Programm. Installer-Bundles sind in [tauri.conf.json](../src-tauri/tauri.conf.json) deaktiviert; Signierung und ein öffentlicher Release-Prozess sind noch nicht eingerichtet. Für einen nativen Debug-Build:

```sh
npm run desktop:build -- --debug
```

## Projektstruktur

| Pfad | Inhalt |
| --- | --- |
| `src/app/app.*` | Ansichten und Anwendungsabläufe |
| `src/app/core/` | Modelle, persistenter Signal-Store, Quellenanbindung, Versions- und Profilvergleich |
| `src/app/shared/` | Gemeinsam verwendete Oberflächenelemente |
| `src-tauri/src/` | Installation, Anmeldung, Java, Downloads, Sicherungen und Spielprozesse |
| `public/assets/` | Landschaften, Markenzeichen und verkleinerte Vorschauen |
| `tests/` | TypeScript-Tests und optionaler UI-Regressionstest |
| `docs/examples/` | Importierbare Beispielprofile |
| `docs/media/` | Für die Projektpräsentation vorbereitete Bilder |

Die Herkunft der Bilder ist in [assets.md](assets.md) dokumentiert.

## Reguläre Prüfungen

```sh
npm test
npm run build
npm run test:native
```

Die TypeScript-Tests prüfen die Versionsauswahl, Kataloggruppierung und den Profilvergleich. Die Rust-Tests prüfen unter anderem Downloadvalidierung, Abhängigkeiten, Startargumente, Anmeldung und Wiederherstellung. Die Anmeldetests verwenden keine echten Konten oder Tokens und greifen nicht auf den Betriebssystem-Kontospeicher zu. Die als `ignored` markierten Integrationstests laufen mit diesem Befehl nicht mit.

Einzelne Bereiche lassen sich gezielt ausführen:

```sh
node --experimental-strip-types --test tests/minecraft-versions.test.ts
node --experimental-strip-types --test tests/profile-sync.test.ts
cargo test --locked --manifest-path src-tauri/Cargo.toml auth::tests
```

### Optionaler UI-Regressionstest

[launcher-ui.spec.mjs](../tests/launcher-ui.spec.mjs) benötigt das Node-Modul `playwright` und installiertes Google Chrome. Playwright ist derzeit keine Projektabhängigkeit. Das Modul separat bereitstellen oder über `SYNCMC_PLAYWRIGHT_MODULE` einen importierbaren Modulpfad angeben.

Nach Start des Entwicklungsservers in einem zweiten Terminal ausführen:

```sh
SYNCMC_TEST_URL=http://127.0.0.1:4200 node tests/launcher-ui.spec.mjs
```

Die Umgebungsvariablen-Syntax dieses Beispiels gilt für eine POSIX-Shell. In PowerShell die Variable zuerst mit `$env:SYNCMC_TEST_URL = 'http://127.0.0.1:4200'` setzen und danach `node tests/launcher-ui.spec.mjs` ausführen.

Der Test verwendet einen isolierten Browserkontext und Testdaten. Native Tauri-Kommandos werden simuliert. Er prüft daher Oberflächenabläufe, aber keine echte Microsoft-Anmeldung, Minecraft-Installation oder Spielausführung. Mit `SYNCMC_TEST_SCREENSHOT` lässt sich ein Screenshotpfad mit Verzeichnis angeben, zum Beispiel `/tmp/syncmc-qa/frontend.png`.

## Optionale Netzwerktests

Diese Tests laden echte Dateien und sind standardmäßig deaktiviert. Sie verändern gezielt Dateien in separaten Testordnern, um Reparaturabläufe zu prüfen. Für die konfigurierbaren Pfade ausschließlich eigene Testverzeichnisse verwenden. Die Standardpfade unter `/private/tmp/` stammen aus der bisherigen macOS-Prüfung; die Fabric-Startplantests verwenden diesen Pfad fest und sind daher kein plattformunabhängiger Testablauf.

### Vanilla-Installation und Reparatur

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml real_minecraft_installation -- --ignored --nocapture
```

Der Test lädt standardmäßig den neuesten Snapshot aus Mojangs Versionsmanifest, prüft die Installation und repariert ein absichtlich beschädigtes Asset. Je nach Version werden mehrere hundert Megabyte geladen; beim bisherigen Test waren es rund 650 MB.

| Variable | Bedeutung | Standard |
| --- | --- | --- |
| `SYNCMC_TEST_VERSION` | Konkrete Minecraft-Version | Neuester Snapshot |
| `SYNCMC_INSTALL_TEST_DIR` | Separates Installationsverzeichnis | `/private/tmp/syncmc-install-smoke` |

### Fabric, Modrinth und gemeinsamer Startplan

Die folgenden drei Befehle **nacheinander** ausführen. Sie teilen sich `/private/tmp/syncmc-fabric-smoke` und bauen aufeinander auf:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml real_fabric_launch_preparation -- --ignored --nocapture
cargo test --locked --manifest-path src-tauri/Cargo.toml real_modrinth_installation -- --ignored --nocapture
cargo test --locked --manifest-path src-tauri/Cargo.toml real_modded_launch_plan -- --ignored --nocapture
```

Der erste Test installiert Minecraft 1.21.1 mit Fabric, prüft ein auffindbares Java 21 sowie die Startargumente und repariert eine beschädigte Loader-Bibliothek. Dabei werden rund 900 MB geladen. Der zweite Test installiert einen echten Modrinth-Mod mit Pflichtabhängigkeit und prüft unter anderem Reparatur, Deaktivierung, Entfernen und Wiederaufnahme eines unterbrochenen Datei-Commits. Der dritte Test validiert den gemeinsamen Startplan mit den tatsächlich installierten Inhalten.

`SYNCMC_FABRIC_TEST_DIR` kann den Ordner ausschließlich für `real_modrinth_installation` überschreiben. Für die gemeinsame Prüfung mit den beiden Startplantests den Standardpfad beibehalten. Keiner dieser Tests startet Minecraft oder authentifiziert ein echtes Konto; synthetische Sitzungswerte werden ausschließlich zum Prüfen der Startargumente verwendet.

### Verwaltete Java-Laufzeit

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml real_managed_java_installation -- --ignored --nocapture
```

Dieser Test lädt ein offizielles Eclipse-Temurin-Paket, prüft Wiederverwendung und repariert eine absichtlich beschädigte Java-Datei.

| Variable | Bedeutung | Standard |
| --- | --- | --- |
| `SYNCMC_JAVA_TEST_DIR` | Separater Datenordner für die Testlaufzeit | `/private/tmp/syncmc-managed-java-smoke` |
| `SYNCMC_JAVA_TEST_MAJOR` | Java-Hauptversion | `21` |

## Prüfergebnisse richtig einordnen

Bei einem Testbericht den geprüften Betriebssystem- und Versionsstand sowie den tatsächlichen Umfang angeben. Reine Tests, simulierte UI-Abläufe und echte Downloads sind kein Nachweis eines authentifizierten Spielstarts. Persönliche Daten, Zugangsdaten und private Dateipfade vor dem Teilen entfernen.

Mojang hat die Minecraft-API-Freigabe am **6. November 2026** bestätigt. Am **7. November 2026** wurde unter macOS ARM64 der echte Ablauf mit Minecraft 1.21.1, Fabric 0.20.1 und verwaltetem Temurin Java 21.0.13 geprüft: anmelden → Testwelt betreten → speichern und Minecraft regulär mit Exit-Code 0 beenden → SyncMC vollständig beenden und neu öffnen → Minecraft ohne neue Microsoft-Anmeldung starten → dieselbe gespeicherte Welt erneut laden. Die Sitzungserneuerung und der Erhalt der Welt sind damit bestätigt. Von der persönlichen Konto- und Spielabnahme bleibt die Abmeldeprüfung offen. Der aktuelle Stand steht in [authentication.md](authentication.md); die Voraussetzungen und Einschränkungen von CurseForge stehen in [content-sources.md](content-sources.md).

## Hinweise zur lokalen Umgebung

Angular wird über [scripts/angular.mjs](../scripts/angular.mjs) mit SQLite als Buildcache gestartet, weil der native LMDB-Cache in der bisherigen Node-/macOS-Umgebung abstürzte. `NG_BUILD_CACHE_STORE` kann diese Vorgabe bei Bedarf überschreiben.

Die drei mitgelieferten Beispielinstanzen enthalten zunächst Beispieldaten. Vor einer echten Installation Platzhalter entfernen, deaktivieren oder durch konkrete Versionen ersetzen. Ein vollständiges Profil mit geprüften Fabric-, Sodium-Extra- und Sodium-Pins liegt in [first-fabric-instance.json](examples/first-fabric-instance.json). Gerätewerte, Zugangsdaten und lokale Installationsnachweise gehören nicht zum exportierten gemeinsamen Profil.
