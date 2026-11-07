# Minecraft-Anmeldung in SyncMC

SyncMC liefert seine eigene öffentliche Microsoft-Client-ID zentral mit der App aus. **Spieler benötigen keine eigene Azure-/Entra-Registrierung.** Die SyncMC-Standard-ID lautet `a653da4d-bfbd-4c80-83d4-5fb1a5272475`; sie ist eine öffentliche Anwendungskennung und kein Geheimnis. Der zentrale Wert steht in [app-config.ts](../src/app/core/app-config.ts). Es ist keine fremde Launcher-ID eingebaut.

## Mit dem Minecraft-Konto anmelden

1. SyncMC als Desktop-App öffnen und unter **Einstellungen → Minecraft-Konto** die Anmeldung starten. Die SyncMC-Standard-ID ist bereits eingetragen.
2. Im geöffneten Systembrowser den angezeigten Benutzer-Code auf der von Microsoft gelieferten Adresse eingeben und das eigene Microsoft-Konto auswählen, das Minecraft Java besitzt bzw. einen gültigen PC-Game-Pass-Zugang hat. Die angezeigte Anwendungsberechtigung prüfen.
3. Nach erfolgreicher Anmeldung erscheint der Java-Profilname in SyncMC. Anschließend kann die vorbereitete Instanz über **Minecraft starten** gestartet werden.

Unter **Minecraft-Konto → Erweitert** lässt sich für eigene Forks oder Entwicklung eine andere öffentliche Client-ID hinterlegen. **SyncMC-Standard verwenden** stellt den ausgelieferten Wert wieder her. Keine Passwörter oder Client Secrets in SyncMC eintragen.

## Aktueller Stand der Freigabe und Prüfung

Die eigene SyncMC-App wurde am **5. November 2026** für persönliche Microsoft-Konten registriert; Public-Client-Flows sind aktiviert. Der Antrag auf Minecraft-API-Zugang wurde am **5. November 2026** erfolgreich eingereicht. **Mojang Enforcement hat per E-Mail vom 6. November 2026 (Betreff „AppID Review Complete (11.06.2026)“) die Genehmigung der eingereichten Anwendung(en) für die API-Allowlist bestätigt.** Die E-Mail nennt keine konkrete Client-ID; die Zuordnung zum SyncMC-Antrag beruht auf dem zuvor eingereichten Antrag.

Vor der Freigabe funktionierte bereits die Microsoft-Geräteanmeldung; der anschließende Zugriff auf Minecraft Services scheiterte damals mit HTTP 403. Am **7. November 2026** wurde die Anmeldung nach der Freigabe mit der SyncMC-Standard-ID erfolgreich abgeschlossen: Microsoft, Xbox Live, XSTS, Minecraft-Spielberechtigung und Java-Profil wurden geprüft; das verbundene Profil erschien in SyncMC.

Anschließend startete SyncMC unter **macOS ARM64** die vorbereitete Instanz mit **Minecraft 1.21.1, Fabric 0.20.1 und verwaltetem Eclipse Temurin Java 21.0.13**. Der Launcher meldete einen laufenden Spielprozess; das bereinigte Protokoll bestätigte das Laden von Fabric sowie die Initialisierung von Texturatlanten, Grafik und Audio. Eine lokale Bildschirmaufnahme vom **7. November 2026 gegen 10:12 Uhr** bestätigte das gerenderte Optionsmenü im Minecraft-Fenster.

Danach wurde eine neue lokale Testwelt angelegt und sichtbar betreten. Nach **Save and Quit to Title** bestätigte das bereinigte Protokoll das Speichern aller Dimensionen; Minecraft wurde über **Quit Game** regulär beendet und SyncMC zeigte Exit-Code 0. Anschließend wurde SyncMC vollständig beendet und als neuer Prozess gestartet. Das verbundene Profil erschien ohne erneute Geräteanmeldung; **Minecraft starten** startete das Spiel erneut erfolgreich. Dieser Start prüfte die Sitzungserneuerung aus dem gespeicherten Refresh-Token einschließlich erneuter Minecraft-Spielberechtigungs- und Profilprüfung. Dieselbe gespeicherte Testwelt ließ sich anschließend erneut öffnen; die wiederhergestellte Szene wurde visuell und der Weltbeitritt im bereinigten Protokoll bestätigt. Von der unten aufgeführten Konto- und Spielabnahme bleibt die Abmeldung offen.

Für diese Registrierung liefert Microsoft `https://www.microsoft.com/link` und einen achtstelligen Benutzer-Code. SyncMC übernimmt die geprüfte Adresse unverändert aus der Providerantwort und öffnet sie nur für einen aktiven, noch gültigen Anmeldeablauf. Die zugehörigen Regressionstests prüfen auch den alternativen Microsoft-Device-Login mit neunstelligem Code.

Der implementierte Ablauf verwendet Microsoft Device Code, Xbox Live, XSTS und anschließend Minecraft Services. Er prüft die Java-Spielberechtigung über `entitlements/license` und das authentifizierte Spielerprofil. Die Lizenzantwort muss `game_minecraft` oder `product_minecraft` enthalten. Eine validierte Sitzung wird vor dem Spielstart geprüft und bei Ablauf erneuert; es gibt keinen Start mit erfundener UUID oder erfundenem Zugriffstoken.

## Was lokal gespeichert wird

- Die öffentliche SyncMC-Client-ID wird mit der App ausgeliefert. Eine optional abweichende Client-ID unter **Erweitert** bleibt eine lokale Geräteeinstellung.
- Der Microsoft-Refresh-Token und das zuletzt bestätigte Profil werden ausschließlich im Betriebssystem-Kontospeicher abgelegt: macOS Schlüsselbund, Windows Anmeldeinformationsverwaltung oder Linux Secret Service.
- Microsoft-, Xbox- und Minecraft-Zugriffstokens sowie der geheime Device Code werden im nativen Rust-Kern verarbeitet. Für den Spielstart wird die Minecraft-Sitzung an den Java-Spielprozess übergeben. Die Weboberfläche erhält keine Tokens, sondern nur einen kurzlebigen Benutzer-Code und nach erfolgreicher Anmeldung Profilnamen und UUID.
- Ein fehlender/gesperrter Kontospeicher erzeugt einen Fehler; es gibt keine unverschlüsselte Ersatzdatei.
- Vor dem Spielstart wird eine abgelaufene Sitzung erneuert. Nach einem App-Neustart wird sie aus dem sicher gespeicherten Refresh-Token aufgebaut.
- **Abmelden** entfernt den lokalen Kontoeintrag und laufende Anmeldeversuche. Es meldet den Benutzer nicht im Systembrowser von Microsoft ab und ist kein serverseitiger Widerruf bereits ausgestellter Tokens.

Die Anmeldung hat maximal einen aktiven Device-Code-Ablauf. Rust erzwingt dessen Polling-Intervall und Ablaufzeit; Abbruch oder Abmeldung während einer Netzwerkantwort kann kein Konto nachträglich wiederherstellen. Protokollfehler geben keine Antwortkörper oder Tokens aus.

## Prüfen

Die automatisierten Tests für die Anmeldung verwenden keine Konten, keine echten Tokens und greifen nicht auf den Betriebssystem-Kontospeicher zu:

```sh
cargo test --manifest-path src-tauri/Cargo.toml auth::tests
```

Stand der persönlichen Abnahme mit der SyncMC-Standard-ID vom **7. November 2026**:

- [x] Echte Anmeldung einschließlich Spielberechtigung und Java-Profil; verbundenes Profil erscheint in SyncMC.
- [x] Authentifizierten Spielprozess starten; Fabric sowie Grafik- und Audioinitialisierung im bereinigten Protokoll bestätigt.
- [x] Gerendertes Spielmenü visuell prüfen: Optionsmenü im Minecraft-Fenster bestätigt.
- [x] Eine neue lokale Testwelt betreten, alle Dimensionen speichern und das Spiel regulär mit Exit-Code 0 beenden.
- [x] SyncMC vollständig beenden und als neuen Prozess öffnen; Sitzung aus dem gespeicherten Refresh-Token erneuern und Minecraft ohne erneute Geräteanmeldung erfolgreich starten.
- [x] Dieselbe gespeicherte Testwelt nach dem Neustart erneut öffnen und visuell prüfen.
- [ ] In SyncMC abmelden und den entfernten Kontostatus prüfen.

Für den Abschluss dieser Konto- und Spielabnahme steht nur noch die Abmeldung aus. Das Konto blieb verbunden; die erneut geöffnete Testwelt wurde pausiert. Ein nach dem App-Neustart angezeigtes gespeichertes Profil allein weist keine erfolgreiche Sitzungserneuerung nach; diese wurde hier durch den erneuten authentifizierten Spielstart bestätigt.

## Anhang: eigene Registrierung für Betreiber und Forks

Die folgenden Schritte betreffen die Pflege der zentralen SyncMC-Registrierung oder eine eigene Fork mit eigener Anwendungskennung. Sie sind **keine Einrichtungsschritte für jeden Spieler**. Die normale SyncMC-App nutzt die oben genannte Standard-ID. Die eigene SyncMC-Registrierung ist bereits angelegt; die Genehmigung des API-Antrags wurde bestätigt. Eigene Forks mit anderer Client-ID benötigen ihre eigene Freigabe.

### Wenn bisher nur ein privates Microsoft-Konto vorhanden ist

Ein privates Konto bei Outlook/Live/Xbox allein stellt noch kein eigenes Entra-Verzeichnis bereit. Der Portalfehler **AADSTS16000** mit dem Tenant **Microsoft Services** und der Anwendung **ADIbizaUX** passt zu diesem Fall: Das Konto kann sich anmelden, besitzt dort aber keinen Verzeichniszugang für Appregistrierungen. Microsoft nennt ein privates Konto ohne zugeordnetes Verzeichnis ausdrücklich als häufige Ursache. Ein vorhandenes eigenes Verzeichnis mit den nötigen Registrierungsrechten kann stattdessen genutzt werden. [Microsoft: Fehlerreferenz](https://learn.microsoft.com/en-us/entra/identity-platform/reference-error-codes), [Verzeichnis für Appentwicklung](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-create-new-tenant).

Ohne vorhandenes Verzeichnis nennt Microsoft den **Azure Free Account** als Einstieg zu einem kostenlosen Entra-Tenant. Auf der [offiziellen Azure-Kontoseite](https://azure.microsoft.com/en-us/pricing/purchase-options/azure-account) **Try Azure for free** wählen, sofern die Neukundenberechtigung vorliegt. Dafür verlangt Microsoft ein Microsoft-/GitHub-Konto, eine Telefonnummer und eine Kreditkarte oder nicht vorausbezahlte Debitkarte. Zur Identitätsprüfung kann eine vorübergehende Autorisierung von etwa 1 USD bzw. dem Gegenwert in Landeswährung erscheinen. Microsoft sagt für den Free Account keine Belastung ohne ausdrücklichen Wechsel auf Pay-as-you-go zu. Ein kostenpflichtiges Microsoft-365-Abonnement ist für diesen Einstieg keine Voraussetzung. [Microsoft: kostenloser Entwicklungs-Tenant](https://learn.microsoft.com/en-us/entra/verified-id/how-to-create-a-free-developer-account), [Azure-Angebot und Voraussetzungen](https://azure.microsoft.com/en-us/pricing/purchase-options/azure-account#faq).

Das Azure-Free-Angebot ist auf Neukunden sowie zunächst 30 Tage bzw. das enthaltene Guthaben begrenzt; daraus folgt keine Zusage dauerhaft kostenloser Azure-Nutzung. Die direkte Erstellung **zusätzlicher Workforce-Tenants** im Entra-Portal ist für kostenlose bzw. Trial-Kunden eingeschränkt und erfordert laut Microsoft einen zahlenden Kundenstatus. Für den ersten Einstieg ist daher die Free-Account-Registrierung maßgeblich. Danach im eigenen Verzeichnis mit der folgenden Appregistrierung fortfahren. [Microsoft: Tenant erstellen](https://learn.microsoft.com/en-us/entra/fundamentals/create-new-tenant).

### Eigene App einmalig registrieren

1. Im [Microsoft Entra Admin Center](https://entra.microsoft.com/) unter **Entra ID → App registrations → New registration** die eigene Anwendung bzw. Fork anlegen. Dafür muss das Microsoft-Konto auf ein Verzeichnis mit den nötigen Registrierungsrechten zugreifen können. Details: [Microsoft: App registrieren](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app).
2. Als unterstützten Kontotyp **Personal Microsoft accounts only** oder **Any Entra ID tenant + Personal Microsoft accounts** wählen. SyncMC verwendet den Microsoft-Consumer-Endpunkt; reine Organisationskonten funktionieren dafür nicht.
3. Unter **Authentication → Advanced settings** die Option **Allow public client flows** einschalten und speichern. Device Code benötigt weder Client Secret noch Redirect-Server. Die genauen Menünamen können sich ändern; Microsoft beschreibt diese Einstellung unter [Enable public client flow](https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration#enable-public-client-flow).
4. Für eine eigene Fork die öffentliche Client-ID zentral konfigurieren oder zum Testen unter **Minecraft-Konto → Erweitert → Microsoft-App-Client-ID** eintragen. Diese lokale Abweichung ist kein notwendiger Schritt für reguläre SyncMC-Nutzer. Die ID ist öffentlich und kein Passwort; keinen Client Secret erzeugen oder eintragen.
5. Nach der eigenen API-Freigabe die Anmeldung in der eigenen Anwendung prüfen. Im Systembrowser öffnet sich die Microsoft-Seite. Dort den angezeigten Benutzer-Code eingeben, das eigene Minecraft-Konto auswählen und die angezeigte Anwendung und Berechtigung prüfen.

SyncMC fordert `XboxLive.signin offline_access` an. Der Login ist ein [Microsoft Device Authorization Grant](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code); `offline_access` ermöglicht die spätere [Sitzungserneuerung](https://learn.microsoft.com/en-us/entra/identity-platform/scopes-oidc#the-offline_access-scope).

### Minecraft-API-Freigabe für die eigene App beantragen

Nach Microsoft folgen Xbox User Token, XSTS und Minecraft Services. Eine erfolgreiche Microsoft-Anmeldung beweist daher noch keinen Minecraft-Zugriff. Für neue Anwendungen verlangt Minecraft einen eigenen Freigabeantrag. Der offizielle [Hilfeartikel zum Java-Edition-API-Prüfverfahren](https://help.minecraft.net/hc/en-us/articles/16254801392141) verlinkt das aktive [AppID-Prüfformular](https://aka.ms/mce-reviewappid). Der SyncMC-Antrag wurde am **5. November 2026** einmal abgesendet; die anschließend angezeigte Dankeseite von Mojang Studios bestätigte zunächst die Einreichung. Die Genehmigung für die API-Allowlist wurde anschließend per E-Mail vom **6. November 2026** bestätigt (siehe Freigabestand oben).

Laut Formular müssen Anwendungen den Spielbesitz prüfen und die Anmeldung vor dem Spielen verlangen. Anwendungen, die Sicherheitsprüfungen umgehen, werden abgelehnt. Neue Anwendungsnamen dürfen die dort genannten Namen **Mojang, Minecraft, Microsoft, Live, Xbox, Discord oder Hypixel** nicht enthalten. Die Anträge werden laut Formular **wöchentlich manuell geprüft**; doppelte Einreichungen beschleunigen dies nicht. Eine bestimmte Bearbeitungsdauer oder Genehmigung ist damit nicht zugesagt.

Das Formular fragt folgende Angaben ab:

1. Bestätigung, dass EULA und [Minecraft-Nutzungsrichtlinien](https://aka.ms/mcusageguidelines) gelesen und verstanden wurden; Ja/Nein-Auswahl.
2. Kontaktadresse: gültige E-Mail-Adresse, die mit den Angaben im Azure-Portal abgeglichen werden kann.
3. Art des Antrags: neue App-ID zur Genehmigung oder Prüfung einer bestehenden App-ID.
4. Anwendungsname.
5. Application-/Client-ID als GUID.
6. Tenant-ID als GUID; für neue Anträge erforderlich.
7. Zugehörige Website oder Domain: URL zur Anwendung, Website oder Marke.
8. Begründung: Überblick über die Anwendung und weshalb sie die API benötigt; eine gültige Begründung ist erforderlich.
9. Weitere Informationen.

Die Bestätigung der EULA und Nutzungsrichtlinien muss der Betreiber nach eigener Prüfung abgeben. Client-ID und Tenant-ID aus der eigenen Registrierung übernehmen; keine IDs anderer Launcher verwenden. SyncMC meldet einen abgelehnten Minecraft-Zugriff mit **HTTP 403** als Fehler der Appregistrierung/API-Freigabe und umgeht die fehlende Freigabe nicht.

Ein Konto muss Minecraft Java besitzen oder einen dafür gültigen PC-Game-Pass-Zugang haben und ein Java-Spielerprofil besitzen. SyncMC ruft sowohl die aktuellen Minecraft-Spielberechtigungen unter `entitlements/license` als auch das authentifizierte Java-Profil ab. Die Lizenzantwort muss `game_minecraft` oder `product_minecraft` enthalten; ein vorhandenes Profil allein genügt nicht. Der aktuelle Lizenzendpunkt berücksichtigt auch Abonnements; die ältere `mcstore`-Abfrage konnte bei Game-Pass-Konten leer sein. Es gibt keinen Start mit erfundener UUID oder einem erfundenen Zugriffstoken.

#### Englischer Begründungsentwurf zur Prüfung

Der folgende Text ist ein Formulierungsbeispiel für eigene Anträge oder Forks, kein Nachweis des genauen eingereichten Wortlauts. Er beschreibt die Implementierung, keinen erfolgreich durchgeführten vollständigen Konto- und Spieltest. Der jeweilige Betreiber muss ihn vor der Verwendung prüfen und bestätigen; Website, Kontaktangaben und Richtlinienbestätigung sind separat zu ergänzen.

> SyncMC is a native desktop launcher for Minecraft: Java Edition, currently under development. It manages local instances with pinned Fabric loader and Modrinth content versions. Minecraft Services access is needed to authenticate legitimate players, verify their current Java Edition entitlement and authenticated player profile, and obtain the session required to launch the game. The application uses its own Microsoft public-client registration with the device-code flow, followed by Xbox Live and XSTS authentication. Gameplay is permitted only with a validated Minecraft session. Microsoft refresh tokens are stored in the operating system's credential store; access tokens are handled in native code and are not exposed to the web interface. SyncMC provides no authentication or game-ownership bypass.

## Technische Primärquellen

- [Minecraft: Java-Edition-API-Prüfverfahren](https://help.minecraft.net/hc/en-us/articles/16254801392141) und [AppID-Prüfformular](https://aka.ms/mce-reviewappid)
- [Microsoft: Device-Code-Protokoll und erwartete Antworten](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)
- [Microsoft: Xbox-Dienste, Microsoft-Token und Xbox User Token](https://learn.microsoft.com/en-us/xbox/gdk/docs/services/fundamentals/s2s-auth-calls/service-authentication/live-website-authentication)
- [Prism Launcher: Xbox User Step](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/steps/XboxUserStep.cpp), [XSTS-Autorisierung](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/steps/XboxAuthorizationStep.cpp), [Spielberechtigungen](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/steps/EntitlementsStep.cpp), [Lizenzantwort](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/Parsers.cpp) und [authentifiziertes Minecraft-Profil](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/steps/MinecraftProfileStep.cpp) als Protokollreferenzen. SyncMC enthält eine eigenständige Rust-Implementierung.
- [Modrinth: Minecraft-Anmeldeschritte und Game-Pass-Besonderheiten](https://support.modrinth.com/en/articles/9038231-minecraft-sign-in-issues)
- [Keyring: native Kontospeicher und Funktionen](https://docs.rs/keyring/3.6.3/keyring/)
