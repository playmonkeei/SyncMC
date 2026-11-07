# Inhaltsquellen: Modrinth und CurseForge

SyncMC trennt Inhaltsquellen vom gemeinsamen Instanzprofil. Ein Pin besteht aus **Quelle, Projekt-ID und konkreter Datei-/Versions-ID**. Gleiche IDs aus verschiedenen Quellen sind unterschiedliche Projekte. Gleichnamige Dateien oder nicht nachgewiesene Doppelinstallationen werden nicht still zusammengeführt.

Die Katalogauswahl startet mit **Alle Anbieter** und durchsucht Modrinth und das eingerichtete CurseForge parallel. Einzelne Anbieter bleiben auswählbar. Jeder Treffer trägt eine sichtbare Quellenkennzeichnung; Versionsauswahl und Installation verwenden genau diese Quelle. Ergebnisse werden abwechselnd nach Anbieter angezeigt, wobei die jeweilige Suchreihenfolge erhalten bleibt. Schnelle Ergebnisse erscheinen bereits, während der andere Anbieter noch lädt. Ohne CurseForge-Einrichtung, im Browser oder bei nicht unterstützten Data Packs bleibt Modrinth nutzbar. Fehler eines Anbieters werden angezeigt, ohne die Treffer des anderen zu verwerfen.

Im gemeinsamen Katalog werden Einträge mit gleichem normalisiertem Titel, gleichem Inhaltstyp und identischem Quellcode-Repository in einer Zeile dargestellt. Darin lässt sich der Anbieter ausdrücklich auswählen. Ein bereits im Profil verwendeter Anbieter bleibt vorausgewählt. Die Beschreibung und Downloadzahl beziehen sich auf den ausgewählten Anbieter; Zahlen werden nicht addiert. Die Auswahl ändert keine Pins oder Dateien. Ist ein in der Ergebnisgruppe zugeordnetes Projekt bereits über den anderen Anbieter im Profil, verhindert der Versionsdialog eine zweite Vormerkung und erklärt den bewussten Quellenwechsel durch vorheriges Entfernen des bisherigen Eintrags.

Diese Gruppierung ist ein Darstellungshinweis, kein Nachweis identischer Dateien oder der Urheberschaft. Gleiche Namen oder IDs allein reichen nicht. Unterstützt werden eindeutige Repository-Wurzeln von GitHub, GitLab, Codeberg und Bitbucket; fehlende, unbekannte oder mehrdeutige Verweise lassen Treffer getrennt. Auch mehrere Projekte desselben Anbieters mit gleichem Titel und Repository werden nicht beliebig einander zugeordnet. CurseForge liefert den Repository-Link im Suchobjekt. Für passende Namenskandidaten lädt SyncMC Modrinths optionale Projektlinks in höchstens einem auf 24 IDs begrenzten Sammelaufruf je Suche nach. Ein Fehler dieses Zusatzaufrufs lässt die ursprünglichen Suchtreffer bestehen; verspätete Antworten verändern keine neuere Suche. Externe Repository-Adressen werden für den Vergleich nur lokal normalisiert und nicht aufgerufen.

## Stand von CurseForge

Die native Anbindung unterstützt Suche, Versionsauswahl und Downloads einschließlich Pflichtabhängigkeiten. Sie benötigt einen eigenen genehmigten API-Schlüssel. Der zulässige Nutzungsumfang für eine öffentliche Veröffentlichung ist noch zu klären. Die separate Mojang-Freigabe für die Minecraft-API wurde am **6. November 2026** bestätigt; die geprüften Konto- und Spielabläufe sind in [authentication.md](authentication.md) dokumentiert.

CurseForge verlangt einen eigenen Antrag für Drittanbieter. Der offizielle [Artikel zum API-Zugang](https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key) verlinkt das [Antragsformular](https://forms.monday.com/forms/dce5ccb7afda9a1c21dab1a1aa1d84eb?r=use1). Der Antrag wird von Overwolf geprüft.

Die [API-Bedingungen](https://support.curseforge.com/support/solutions/articles/9000207405-curseforge-3rd-party-api-terms-and-conditions) enthalten unter anderem Beschränkungen zur Schlüsselweitergabe und zum Speichern beziehungsweise Zwischenspeichern von API-Daten. Vor einer öffentlichen Veröffentlichung müssen insbesondere der Betrieb als Mehrquellen-Launcher, gespeicherte Versionspins, Profilübertragung, lokale Sicherungen und der Umgang mit dem Schlüssel ausdrücklich mit CurseForge geklärt werden. Die technische Implementierung ist keine erteilte Nutzungsfreigabe.

## Lokaler Schlüssel und Downloads

Die Desktop-App kann einen eigenen genehmigten Schlüssel lokal im Betriebssystem-Kontospeicher hinterlegen. Das Eingabefeld wird nach dem Speicherversuch geleert. Die Oberfläche erhält beim späteren Prüfen ausschließlich den Verbindungsstatus; der Schlüssel wird weder in Geräteeinstellungen noch Profilen, Exporten oder Sicherungen gespeichert. Es wird kein fremder Launcher-Key übernommen und kein Schlüssel im Repository ausgeliefert.

Anfragen gehen direkt an die offizielle CurseForge-API. Downloads verwenden ausschließlich von der API gelieferte, geprüfte offizielle CDN-Adressen. Die Authentifizierung erfolgt über den Header `x-api-key`, nicht über die URL. CurseForge hat die Pflicht zur Download-Authentifizierung für den **16. Juli 2026** angekündigt; siehe [offizielle Download-Umstellung](https://blog.curseforge.com/introducing-api-key-authentication-for-curseforge-file-downloads/).

Die [Drittanbieter-Freigabe des Autors](https://support.curseforge.com/support/solutions/articles/9000207877) wird berücksichtigt. Eine verweigerte Freigabe oder fehlende Downloadadresse führt zu einer verständlichen Meldung. SyncMC errechnet keine Ersatzadresse und wechselt nicht auf ein inoffizielles Downloadverfahren.

## Prüfsummen und Abhängigkeiten

Modrinth veröffentlicht für die verwendeten Dateien SHA-512. CurseForge führt in seiner [REST-Dokumentation](https://docs.curseforge.com/rest-api/) SHA-1 und MD5 als Datei-Hashverfahren auf. SyncMC benötigt für CurseForge die SHA-1-Prüfsumme aus den offiziellen Dateimetadaten und prüft außerdem die Dateigröße. MD5 allein genügt nicht.

Nach einem erfolgreichen CurseForge-Download berechnet SyncMC zusätzlich eine eigene SHA-512-Prüfsumme für den lokalen Installationsnachweis. Diese wird nicht als von CurseForge veröffentlichte Prüfsumme ausgegeben. Der Profilvergleich unterscheidet diese lokale Prüfspur von der gemeinsamen Quelle-/Datei-ID und dem offiziellen SHA-1-Pin.

Pflichtabhängigkeiten werden innerhalb ihrer jeweiligen Quelle aufgelöst. Explizite Versionen bleiben gepinnt; Konflikte, inkompatible Dateien und gesperrte Downloads brechen die Installation vor der Änderung des aktiven Instanzstands ab. Der vorhandene Sicherungs- und Wiederherstellungsablauf gilt auch für gemischte Instanzen.

Die aktuelle Minecraft- und Loader-Kompatibilität wird aus den CurseForge-Dateimetadaten geprüft. CurseForge liefert keine durchgehend belastbare Kennzeichnung für reine Server-Mods und bestätigt bei Shadern keine bestimmte Shader-Engine. Eine passende Minecraft-Version allein beweist daher weder Client-Kompatibilität noch die Eignung für Iris oder OptiFine. Der vorhandene Fabric-Startweg bleibt die erste unterstützte Modinstallation; die zusätzliche Quelle installiert keinen Forge-/NeoForge-Loader.

## Browser-Vorschau

Modrinth funktioniert weiterhin über den lokalen Entwicklungsproxy. CurseForge-Schlüssel und authentifizierte Anfragen gehören zum nativen Kern; die Browser-Vorschau stellt keinen öffentlichen CurseForge-Proxy bereit.

## Prüfung

Die Quellen- und Kataloglogik lässt sich ohne echten Provider-Schlüssel prüfen:

```sh
node --experimental-strip-types --test tests/catalog-groups.test.ts
cargo test --locked --manifest-path src-tauri/Cargo.toml curseforge::tests
```

Der optionale UI-Test simuliert die Tauri-Kommandos. Er prüft unter anderem die explizite Anbieterwahl, getrennte Projekt-IDs, unveränderte Pins und das Verhalten bei Teilausfällen. Einrichtung und Grenzen dieser Prüfung stehen in der [Entwicklungsanleitung](development.md#optionaler-ui-regressionstest).

Echte CurseForge-Downloads erfordern einen eigenen genehmigten Schlüssel und passende Anbieterfreigaben. Ein erfolgreicher API-Aufruf oder Download bestätigt weder eine Nutzungsfreigabe für die öffentliche Verteilung noch einen authentifizierten Minecraft-Spielstart.
