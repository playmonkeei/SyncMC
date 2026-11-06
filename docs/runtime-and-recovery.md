# Java, Sicherungen und Profilabgleich

## Java ohne manuelle Installation

Die Desktop-App kann die von einer installierten Minecraft-Version benötigte Java-Hauptversion über **Java automatisch einrichten** herunterladen. In den Einstellungen lässt sich ebenfalls eine Hauptversion wählen. SyncMC verwendet Eclipse Temurin aus der offiziellen [Adoptium-API](https://adoptium.net/installation/ci-scripts); das Paket muss zur Betriebssystem- und Prozessorarchitektur passen.

Die Installation liegt ausschließlich im SyncMC-App-Datenordner unter `java-runtimes/`. Sie verändert weder das System-Java noch `PATH` oder `JAVA_HOME`. Der Download wird anhand von Größe und SHA-256 geprüft, zunächst in einem Arbeitsordner entpackt und erst nach erfolgreicher Java-Prüfung übernommen. Fehler werden angezeigt; ein fehlgeschlagener Download gilt nicht als fertige Installation.

Ein ausdrücklich eingetragener Java-Pfad bleibt eine lokale Geräteeinstellung. Die Automatik verwendet verwaltete oder bereits vorhandene passende Versionen. Der Spielstart prüft weiterhin Hauptversion und Architektur. Die Java-Einrichtung startet Minecraft nicht.

## Instanzsicherungen

Vor Änderungen an installierten Inhalten sichert der native Kern den bisherigen lokalen Instanzordner. Eine Sicherung kann außerdem ausdrücklich in der Instanzverwaltung erstellt werden. Sie enthält das Instanzprofil mit genauen Versionspins sowie die vorhandenen Dateien dieser Instanz, einschließlich Welten, Konfiguration und Mods. Gemeinsam verwendete Bibliotheken, Assets und Download-Caches werden nicht dupliziert.

Die Sicherungen liegen lokal im SyncMC-Datenordner und lassen sich in der App auflisten und wiederherstellen. Vor einer Wiederherstellung entsteht eine neue Rettungssicherung des aktuellen Stands. Während Minecraft startet oder läuft, sind Dateiänderungen und Wiederherstellungen gesperrt. Eine Wiederherstellung, deren gespeicherte Minecraft-/Loader-Konfiguration nicht zum Installationsnachweis passt, erfordert eine erneute Installation beziehungsweise Prüfung vor dem Start.

Eine Instanzsicherung darf höchstens 20 GiB und 200.000 Dateien enthalten. Überschreitungen, symbolische Links und Kopierfehler brechen den Vorgang ab; eine unvollständige Kopie erscheint nicht als fertige Sicherung. Alte Instanzsicherungen werden nicht automatisch entfernt.

Mod-Updates werden einzeln ausgewählt. Die Updateprüfung berücksichtigt zur Instanz passende Release-Versionen; Beta- und Alpha-Versionen werden dabei nicht automatisch vorgeschlagen. Der native Installationsablauf prüft weiterhin die Dateien und Pflichtabhängigkeiten. Die gewünschten Pins im Frontend werden erst nach einem erfolgreichen Update übernommen.

## Profile zwischen Geräten übertragen

Unter **Geräte & Sync** lassen sich Profile als JSON exportieren und vor dem Import vergleichen. Der Datei-Abgleich arbeitet ohne Cloudverbindung:

- Neue IDs ergänzen die lokalen Instanzen.
- Gleiche IDs mit abweichenden Profilen benötigen eine ausdrückliche Entscheidung für den lokalen oder importierten Stand. Die Vorschau benennt die abweichenden Bereiche.
- Nur lokal vorhandene Instanzen bleiben erhalten; fehlende Einträge in einer Importdatei werden nicht als Löschanweisung behandelt.
- Ein während der Vorschau geänderter lokaler Stand verhindert die Übernahme. Die Datei muss erneut verglichen werden.
- Gerätewerte wie Java-Pfad, RAM und Microsoft-App-Override bleiben lokal. Zugangsdaten werden nicht exportiert.
- Ein importierter Downloadstatus ist kein Installationsnachweis. Geänderte Pins müssen auf diesem Gerät erneut heruntergeladen und geprüft werden.

In der Desktop-App werden Exporte im SyncMC-App-Datenordner unter `exports/` gespeichert. Die App zeigt den fertigen Dateipfad und bietet **Exportordner öffnen** an. Jeder Export erhält einen eindeutigen Namen; vorhandene Dateien bleiben erhalten. Profillisten, einzelne Profilsicherungen und Wiederherstellungs-Rohdaten verwenden denselben nativen Exportweg. Die Browser-Vorschau verwendet weiterhin einen normalen Dateidownload.

Wiederherstellungen, das Löschen einer Profilsicherung und erzwungenes Beenden werden in einem eigenen App-Dialog bestätigt. **Abbrechen**, Escape und ein Klick außerhalb des Dialogs führen die Aktion nicht aus. Der Dialog funktioniert auch in der nativen WebView und ist nicht von Browser-Bestätigungsfenstern abhängig.

Vor einer Übernahme wird eine **Profilsicherung** im lokalen Anwendungsspeicher angelegt. Diese kleinen Sicherungen enthalten die gemeinsame Profilliste, keine Welten oder Moddateien. Sie lassen sich exportieren oder mit einer vorherigen Rettungssicherung wiederherstellen. Änderungen werden abgebrochen, wenn die Sicherung oder der neue Profilstand nicht persistent gespeichert werden kann. Die App entfernt alte Profilsicherungen nicht automatisch.

Der automatische Dropbox-Abgleich ist weiterhin nicht angebunden. Der Datei-Abgleich liefert die gemeinsame Validierung und Entscheidungslogik für eine spätere Cloudanbindung.

## Prüfung

Die reinen Vergleichstests laufen mit:

```sh
node --experimental-strip-types --test tests/profile-sync.test.ts
```

Native Download-, Dateisicherungs- und Wiederherstellungstests liegen bei den jeweiligen Rust-Modulen. Ein echter Minecraft-Spieltest benötigt weiterhin die ausstehende Mojang-Freigabe; die Java- und Sicherungsabläufe lassen sich unabhängig davon prüfen.

Der optionale echte Java-Test lädt das offizielle Paket in einen isolierten Testordner und prüft Wiederverwendung sowie Reparatur einer dort absichtlich beschädigten Java-Datei:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml real_managed_java_installation -- --ignored --nocapture
```

Standardpfad ist `/private/tmp/syncmc-managed-java-smoke`. `SYNCMC_JAVA_TEST_DIR` darf nur auf einen separaten Testordner zeigen, da der Test seine dortige Java-Datei gezielt beschädigt. Weitere Testbefehle, Plattformvoraussetzungen und die Grenzen der simulierten UI-Prüfung stehen in der [Entwicklungsanleitung](development.md).
