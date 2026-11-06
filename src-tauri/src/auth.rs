//! Microsoft personal-account device flow, Xbox/XSTS and Minecraft authentication.
//! Secrets stay in Rust and the operating system's credential store. See docs/authentication.md.

use keyring::Entry;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const OAUTH_BASE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const SCOPES: &str = "XboxLive.signin offline_access";
const KEYRING_SERVICE: &str = "dev.syncmc.desktop.minecraft";
const KEYRING_USER: &str = "active-account-v1";
const MAX_RESPONSE_BYTES: usize = 1_048_576;

static HTTP: OnceLock<Result<Client, String>> = OnceLock::new();
static STATE: OnceLock<Mutex<AuthState>> = OnceLock::new();

#[derive(Default)]
struct AuthState {
    generation: u64,
    pending: Option<PendingFlow>,
    cached: Option<CachedSession>,
}

struct PendingFlow {
    id: String,
    client_id: String,
    device_code: String,
    verification_uri: String,
    expires: Instant,
    next_poll: Instant,
    interval: Duration,
    in_flight: bool,
}

#[derive(Debug, PartialEq)]
enum PollPermit {
    Ready,
    Wait,
    Expired,
}

impl PendingFlow {
    fn reserve_poll(&mut self, now: Instant) -> PollPermit {
        if now >= self.expires {
            return PollPermit::Expired;
        }
        if self.in_flight || now < self.next_poll {
            return PollPermit::Wait;
        }
        self.in_flight = true;
        self.next_poll = now + self.interval;
        PollPermit::Ready
    }

    fn retry_after(&mut self, now: Instant, slow_down: bool) {
        self.in_flight = false;
        if slow_down {
            self.interval += Duration::from_secs(5);
        }
        self.next_poll = now + self.interval;
    }
}

impl AuthState {
    fn is_current(&self, flow_id: &str, generation: u64) -> bool {
        self.generation == generation
            && self.pending.as_ref().is_some_and(|flow| flow.id == flow_id)
    }

    fn login_url(&self, now: Instant) -> Result<&str, String> {
        let flow = self
            .pending
            .as_ref()
            .filter(|flow| flow.expires > now)
            .ok_or("Keine aktive Microsoft-Anmeldung. Bitte die Anmeldung erneut starten.")?;
        if !verified_login_url(&flow.verification_uri) {
            return Err(
                "Die aktive Anmeldung enthält keine vertrauenswürdige Microsoft-Seite.".into(),
            );
        }
        Ok(&flow.verification_uri)
    }
}

struct CachedSession {
    session: MinecraftSession,
    expires: Instant,
}

/// Deliberately not Serialize/Debug: this value must never cross the IPC boundary or enter logs.
#[derive(Clone)]
pub struct MinecraftSession {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub client_id: String,
    pub xuid: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub name: String,
    pub uuid: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginChallenge {
    flow_id: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginPoll {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<AccountSummary>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredCredential {
    client_id: String,
    refresh_token: String,
    account: AccountSummary,
}

struct OAuthToken {
    access_token: String,
    refresh_token: Option<String>,
}

struct ResolvedAccount {
    session: MinecraftSession,
    lifetime: Duration,
}

fn state() -> &'static Mutex<AuthState> {
    STATE.get_or_init(|| Mutex::new(AuthState::default()))
}

fn http() -> Result<&'static Client, String> {
    HTTP.get_or_init(|| {
        Client::builder()
            .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Der Anmeldedienst konnte nicht vorbereitet werden.".into())
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn client_id(value: &str) -> Result<String, String> {
    let id = uuid::Uuid::parse_str(value.trim()).map_err(|_| {
        "Bitte zuerst die eigene Microsoft-Anwendungs-ID in den Einstellungen hinterlegen."
            .to_owned()
    })?;
    if id.is_nil() {
        return Err("Eine gültige eigene Microsoft-Anwendungs-ID wird benötigt.".into());
    }
    Ok(id.hyphenated().to_string())
}

fn verified_login_url(value: &str) -> bool {
    if value.len() > 2_048 || value.trim() != value || value.chars().any(char::is_control) {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    matches!(
        (url.host_str(), url.path()),
        (
            Some("microsoft.com" | "www.microsoft.com"),
            "/devicelogin" | "/link"
        ) | (
            Some("login.microsoftonline.com"),
            "/common/oauth2/deviceauth" | "/consumers/oauth2/deviceauth"
        )
    )
}

fn safe_account(value: &Value) -> Result<AccountSummary, String> {
    let name = value["name"].as_str().unwrap_or_default();
    let id = value["id"].as_str().unwrap_or_default();
    if name.is_empty()
        || name.len() > 16
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || id.len() != 32
        || !id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Minecraft hat kein gültiges Java-Spielerprofil geliefert.".into());
    }
    Ok(AccountSummary {
        name: name.into(),
        uuid: id.to_ascii_lowercase(),
    })
}

fn secret(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65_536 && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| "Der Anmeldedienst hat eine unvollständige Antwort geliefert.".into())
}

async fn json_response(mut response: reqwest::Response) -> Result<(StatusCode, Value), String> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Die Antwort des Anmeldedienstes konnte nicht gelesen werden.")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("Die Antwort des Anmeldedienstes ist zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    // HTTP errors need not contain JSON. Never show provider bodies: they can contain tokens.
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() && !value.is_object() {
        return Err("Der Anmeldedienst hat keine gültige Antwort geliefert.".into());
    }
    Ok((status, value))
}

fn oauth_error(value: &Value) -> String {
    match value["error"].as_str().unwrap_or_default() {
        "invalid_client" | "unauthorized_client" => "Microsoft akzeptiert die Anwendungs-ID nicht. Prüfe persönliche Microsoft-Konten und die aktivierten Public-Client-Flows in der Appregistrierung.",
        "invalid_scope" => "Die Microsoft-App ist nicht für den angefragten Xbox-Zugriff konfiguriert.",
        "authorization_declined" | "access_denied" => "Die Microsoft-Anmeldung wurde abgelehnt oder abgebrochen.",
        "invalid_grant" | "interaction_required" => "Die Microsoft-Sitzung ist abgelaufen. Bitte erneut anmelden.",
        "expired_token" | "code_expired" => "Der Anmeldecode ist abgelaufen. Bitte die Anmeldung erneut starten.",
        _ => "Die Microsoft-Anmeldung ist fehlgeschlagen. Prüfe die App-Konfiguration und versuche es erneut.",
    }.into()
}

fn service_error(stage: &str, status: StatusCode, value: &Value) -> String {
    let code = value["XErr"]
        .as_u64()
        .or_else(|| value["XErr"].as_str()?.parse().ok());
    match code {
        Some(2_148_916_233) => return "Für dieses Microsoft-Konto fehlt ein Xbox-Profil. Richte es zuerst bei Xbox ein.".into(),
        Some(2_148_916_238) => return "Dieses Xbox-Konto benötigt eine eingerichtete Familiengruppe und die erforderliche Zustimmung.".into(),
        Some(2_148_916_236) => return "Xbox benötigt für dieses Konto eine Altersbestätigung.".into(),
        Some(2_148_916_235) => return "Xbox Live ist für dieses Konto oder diese Region nicht verfügbar.".into(),
        Some(2_148_916_227) => return "Dieses Xbox-Konto wurde gesperrt. Bitte den Xbox-Kontostatus prüfen.".into(),
        _ => {}
    }
    if stage == "Minecraft-Anmeldung" && status == StatusCode::FORBIDDEN {
        return "Minecraft verweigert den API-Zugriff (HTTP 403). Prüfe die eigene Microsoft-Appregistrierung und deren Freigabe für Minecraft Services; eine erfolgreiche Microsoft-Anmeldung allein genügt nicht.".into();
    }
    if stage == "Minecraft-Profil" && status == StatusCode::NOT_FOUND {
        return "Dieses Konto hat kein nutzbares Minecraft-Java-Profil. Prüfe Spielbesitz bzw. PC-Game-Pass und richte das Java-Profil auf minecraft.net ein.".into();
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return "Der Anmeldedienst erhält gerade zu viele Anfragen. Bitte später erneut versuchen."
            .into();
    }
    format!("{stage} fehlgeschlagen (HTTP {}). Bitte die Kontoeinstellungen und Internetverbindung prüfen.", status.as_u16())
}

async fn post_json(url: &str, body: Value, stage: &str) -> Result<Value, String> {
    let response = http()?
        .post(url)
        .header("Accept", "application/json")
        .header("x-xbl-contract-version", "1")
        .json(&body)
        .send()
        .await
        .map_err(|_| format!("{stage}: Der Anmeldedienst ist nicht erreichbar."))?;
    let (status, value) = json_response(response).await?;
    if !status.is_success() {
        return Err(service_error(stage, status, &value));
    }
    Ok(value)
}

async fn get_minecraft(path: &str, token: &str, stage: &str) -> Result<Value, String> {
    let response = http()?
        .get(format!("https://api.minecraftservices.com/{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| format!("{stage}: Minecraft Services ist nicht erreichbar."))?;
    let (status, value) = json_response(response).await?;
    if !status.is_success() {
        return Err(service_error(stage, status, &value));
    }
    Ok(value)
}

fn check_entitlements(value: &Value) -> Result<(), String> {
    let items = value["items"]
        .as_array()
        .ok_or("Die Minecraft-Spielberechtigung konnte nicht geprüft werden.")?;
    if items.iter().any(|item| {
        matches!(
            item["name"].as_str(),
            Some("game_minecraft" | "product_minecraft")
        )
    }) {
        return Ok(());
    }
    Err("Für dieses Konto liegt keine aktuelle Minecraft-Java-Spielberechtigung vor. Prüfe Spielbesitz oder einen gültigen PC-Game-Pass-Zugang.".into())
}

async fn resolve_minecraft(ms_token: &str, client_id: &str) -> Result<ResolvedAccount, String> {
    let xbox = post_json("https://user.auth.xboxlive.com/user/authenticate", json!({
        "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_token}")},
        "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"
    }), "Xbox-Anmeldung").await?;
    let user_hash = secret(&xbox["DisplayClaims"]["xui"][0], "uhs")?;
    let xbox_token = secret(&xbox, "Token")?;
    let xsts = post_json(
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        json!({
            "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbox_token]},
            "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"
        }),
        "Xbox-Autorisierung",
    )
    .await?;
    if secret(&xsts["DisplayClaims"]["xui"][0], "uhs")? != user_hash {
        return Err("Xbox hat während der Anmeldung ein abweichendes Konto geliefert.".into());
    }
    let xuid = xsts["DisplayClaims"]["xui"][0]["xid"]
        .as_str()
        .or_else(|| xbox["DisplayClaims"]["xui"][0]["xid"].as_str())
        .filter(|id| id.len() <= 30 && id.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_default()
        .to_owned();
    let xsts_token = secret(&xsts, "Token")?;
    let minecraft = post_json(
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({
            "identityToken": format!("XBL3.0 x={user_hash};{xsts_token}")
        }),
        "Minecraft-Anmeldung",
    )
    .await?;
    let token = secret(&minecraft, "access_token")?;
    let expires = minecraft["expires_in"]
        .as_u64()
        .filter(|s| *s > 0 && *s <= 172_800)
        .ok_or("Minecraft hat keine gültige Sitzungsdauer geliefert.")?;
    // The current license endpoint includes subscription licenses; the older mcstore
    // endpoint can omit Game Pass entitlements. A profile alone does not prove a license.
    let entitlements = get_minecraft(
        &format!("entitlements/license?requestId={}", uuid::Uuid::new_v4()),
        &token,
        "Minecraft-Spielberechtigung",
    )
    .await?;
    check_entitlements(&entitlements)?;
    let profile = get_minecraft("minecraft/profile", &token, "Minecraft-Profil").await?;
    let account = safe_account(&profile)?;
    Ok(ResolvedAccount {
        session: MinecraftSession {
            name: account.name,
            uuid: account.uuid,
            access_token: token,
            client_id: client_id.into(),
            xuid,
        },
        lifetime: Duration::from_secs(expires.saturating_sub(120)),
    })
}

async fn read_credential() -> Result<Option<StoredCredential>, String> {
    tokio::task::spawn_blocking(|| {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|_| "Der sichere Kontospeicher ist nicht verfügbar.")?;
        let stored = match entry.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(_) => return Err("Auf den sicheren Kontospeicher kann nicht zugegriffen werden. Bitte Schlüsselbund/Anmeldeinformationsverwaltung entsperren.".into()),
        };
        let credential: StoredCredential = serde_json::from_str(&stored)
            .map_err(|_| "Die gespeicherte Sitzung ist ungültig. Bitte abmelden und erneut anmelden.")?;
        client_id(&credential.client_id)?;
        if credential.refresh_token.is_empty() || credential.refresh_token.len() > 65_536 {
            return Err("Die gespeicherte Sitzung ist ungültig. Bitte erneut anmelden.".into());
        }
        safe_account(&json!({"name": credential.account.name, "id": credential.account.uuid}))?;
        Ok(Some(credential))
    }).await.map_err(|_| "Der sichere Kontospeicher konnte nicht gelesen werden.".to_owned())?
}

async fn save_credential(value: StoredCredential) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|_| "Der sichere Kontospeicher ist nicht verfügbar.")?;
        let serialized = serde_json::to_string(&value).map_err(|_| "Die Sitzung konnte nicht vorbereitet werden.")?;
        entry.set_password(&serialized)
            .map_err(|_| "Die Sitzung konnte nicht sicher gespeichert werden. Bitte den Betriebssystem-Schlüsselbund entsperren; es wird keine unverschlüsselte Ersatzdatei angelegt.".to_owned())
    }).await.map_err(|_| "Der sichere Kontospeicher konnte nicht geschrieben werden.".to_owned())?
}

async fn delete_credential() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|_| "Der sichere Kontospeicher ist nicht verfügbar.")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Die gespeicherte Sitzung konnte nicht entfernt werden. Bitte den Betriebssystem-Schlüsselbund entsperren.".into()),
        }
    }).await.map_err(|_| "Der sichere Kontospeicher konnte nicht geöffnet werden.".to_owned())?
}

#[tauri::command]
pub async fn begin_microsoft_login(client_id: String) -> Result<LoginChallenge, String> {
    let client_id = self::client_id(&client_id)?;
    let generation = {
        let mut state = state().lock().await;
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
        state.generation
    };
    let response = http()?
        .post(format!("{OAUTH_BASE}/devicecode"))
        .form(&[("client_id", client_id.as_str()), ("scope", SCOPES)])
        .send()
        .await
        .map_err(|_| "Microsoft ist für die Anmeldung nicht erreichbar.")?;
    let (status, value) = json_response(response).await?;
    if !status.is_success() {
        return Err(oauth_error(&value));
    }
    let (pending, challenge) = challenge_from_response(client_id, &value, Instant::now())?;
    let mut state = state().lock().await;
    if state.generation != generation {
        return Err("Die Anmeldung wurde inzwischen abgebrochen oder ersetzt.".into());
    }
    state.pending = Some(pending);
    Ok(challenge)
}

fn challenge_from_response(
    client_id: String,
    value: &Value,
    now: Instant,
) -> Result<(PendingFlow, LoginChallenge), String> {
    let device_code = secret(value, "device_code")?;
    let user_code = secret(value, "user_code")?;
    if user_code.len() > 40 {
        return Err("Microsoft hat keinen gültigen Anmeldecode geliefert.".into());
    }
    let uri = value["verification_uri"].as_str().unwrap_or_default();
    if !verified_login_url(uri) {
        return Err("Microsoft hat keine vertrauenswürdige Anmeldeseite geliefert.".into());
    }
    let expires_in = value["expires_in"]
        .as_u64()
        .filter(|s| *s > 0 && *s <= 3600)
        .ok_or("Microsoft hat keine gültige Ablaufzeit geliefert.")?;
    let interval = value["interval"].as_u64().unwrap_or(5).clamp(5, 60);
    let flow_id = uuid::Uuid::new_v4().to_string();
    let pending = PendingFlow {
        id: flow_id.clone(),
        client_id,
        device_code,
        verification_uri: uri.into(),
        expires: now + Duration::from_secs(expires_in),
        next_poll: now + Duration::from_secs(interval),
        interval: Duration::from_secs(interval),
        in_flight: false,
    };
    let challenge = LoginChallenge {
        flow_id,
        user_code,
        verification_uri: uri.into(),
        expires_in,
        interval,
    };
    Ok((pending, challenge))
}

fn poll(status: &'static str) -> LoginPoll {
    LoginPoll {
        status,
        account: None,
    }
}

async fn release_poll(flow_id: &str) {
    if let Some(flow) = state()
        .lock()
        .await
        .pending
        .as_mut()
        .filter(|f| f.id == flow_id)
    {
        flow.in_flight = false;
    }
}

#[tauri::command]
pub async fn poll_microsoft_login(flow_id: String) -> Result<LoginPoll, String> {
    let (client_id, device_code, generation) = {
        let mut state = state().lock().await;
        let generation = state.generation;
        let Some(flow) = state.pending.as_mut().filter(|f| f.id == flow_id) else {
            return Ok(poll("expired"));
        };
        match flow.reserve_poll(Instant::now()) {
            PollPermit::Expired => {
                state.pending = None;
                return Ok(poll("expired"));
            }
            PollPermit::Wait => return Ok(poll("pending")),
            PollPermit::Ready => {}
        }
        (flow.client_id.clone(), flow.device_code.clone(), generation)
    };
    let result = async {
        let response = http()?
            .post(format!("{OAUTH_BASE}/token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id.as_str()),
                ("device_code", device_code.as_str()),
            ])
            .send()
            .await
            .map_err(|_| "Microsoft ist für die Anmeldung nicht erreichbar.")?;
        json_response(response).await
    }
    .await;
    let (status, value) = match result {
        Ok(value) => value,
        Err(error) => {
            release_poll(&flow_id).await;
            return Err(error);
        }
    };
    if !status.is_success() {
        let mut state = state().lock().await;
        let Some(flow) = state.pending.as_mut().filter(|f| f.id == flow_id) else {
            return Ok(poll("expired"));
        };
        flow.in_flight = false;
        match value["error"].as_str().unwrap_or_default() {
            "authorization_pending" => {
                flow.retry_after(Instant::now(), false);
                return Ok(poll("pending"));
            }
            "slow_down" => {
                flow.retry_after(Instant::now(), true);
                return Ok(poll("slow_down"));
            }
            "expired_token" | "code_expired" => {
                state.pending = None;
                return Ok(poll("expired"));
            }
            _ => {
                state.pending = None;
                return Err(oauth_error(&value));
            }
        }
    }
    let resolved = async {
        let token = OAuthToken {
            access_token: secret(&value, "access_token")?,
            refresh_token: Some(secret(&value, "refresh_token")?),
        };
        let resolved = resolve_minecraft(&token.access_token, &client_id).await?;
        Ok::<_, String>((token, resolved))
    }
    .await;
    let (token, resolved) = match resolved {
        Ok(value) => value,
        Err(error) => {
            let mut state = state().lock().await;
            if state.pending.as_ref().is_some_and(|f| f.id == flow_id) {
                state.pending = None;
            }
            return Err(error);
        }
    };
    let mut state = state().lock().await;
    // Cancellation or logout during a network request must never resurrect an account.
    if !state.is_current(&flow_id, generation) {
        return Ok(poll("expired"));
    }
    let account = AccountSummary {
        name: resolved.session.name.clone(),
        uuid: resolved.session.uuid.clone(),
    };
    state.pending = None;
    save_credential(StoredCredential {
        client_id,
        refresh_token: token
            .refresh_token
            .ok_or("Microsoft hat keine erneuerbare Sitzung geliefert.")?,
        account: account.clone(),
    })
    .await?;
    state.cached = Some(CachedSession {
        session: resolved.session,
        expires: Instant::now() + resolved.lifetime,
    });
    Ok(LoginPoll {
        status: "complete",
        account: Some(account),
    })
}

#[tauri::command]
pub async fn cancel_microsoft_login(flow_id: String) -> Result<(), String> {
    let mut state = state().lock().await;
    if state.pending.as_ref().is_some_and(|f| f.id == flow_id) {
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_minecraft_account() -> Result<Option<AccountSummary>, String> {
    let state = state().lock().await;
    if let Some(cached) = &state.cached {
        return Ok(Some(AccountSummary {
            name: cached.session.name.clone(),
            uuid: cached.session.uuid.clone(),
        }));
    }
    Ok(read_credential().await?.map(|c| c.account))
}

#[tauri::command]
pub async fn logout_minecraft_account() -> Result<(), String> {
    let mut state = state().lock().await;
    state.generation = state.generation.wrapping_add(1);
    state.pending = None;
    state.cached = None;
    delete_credential().await
}

/// Resolves a current session immediately before launching. No session is returned to JavaScript.
pub async fn minecraft_session() -> Result<MinecraftSession, String> {
    let mut state = state().lock().await;
    if let Some(cached) = &state.cached {
        if cached.expires > Instant::now() {
            return Ok(cached.session.clone());
        }
    }
    let stored = read_credential()
        .await?
        .ok_or("Bitte zuerst mit deinem Minecraft-Konto anmelden.")?;
    let response = http()?.post(format!("{OAUTH_BASE}/token"))
        .form(&[("grant_type", "refresh_token"), ("client_id", stored.client_id.as_str()), ("refresh_token", stored.refresh_token.as_str()), ("scope", SCOPES)])
        .send().await.map_err(|_| "Die Microsoft-Sitzung konnte nicht erneuert werden. Bitte die Internetverbindung prüfen.")?;
    let (status, value) = json_response(response).await?;
    if !status.is_success() {
        return Err(oauth_error(&value));
    }
    let token = OAuthToken {
        access_token: secret(&value, "access_token")?,
        refresh_token: value
            .get("refresh_token")
            .map(|_| secret(&value, "refresh_token"))
            .transpose()?,
    };
    let resolved = resolve_minecraft(&token.access_token, &stored.client_id).await?;
    let account = AccountSummary {
        name: resolved.session.name.clone(),
        uuid: resolved.session.uuid.clone(),
    };
    if account.uuid != stored.account.uuid {
        return Err(
            "Die erneuerte Sitzung gehört zu einem anderen Minecraft-Konto. Bitte erneut anmelden."
                .into(),
        );
    }
    save_credential(StoredCredential {
        client_id: stored.client_id,
        refresh_token: token.refresh_token.unwrap_or(stored.refresh_token),
        account,
    })
    .await?;
    let session = resolved.session.clone();
    state.cached = Some(CachedSession {
        session: resolved.session,
        expires: Instant::now() + resolved.lifetime,
    });
    Ok(session)
}

async fn open_browser_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = tokio::process::Command::new("open");
        c.arg("--").arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = tokio::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c.creation_flags(0x08000000);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut c = tokio::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await;
    match status {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => Err("Der Systembrowser konnte nicht geöffnet werden. Öffne die im Dialog angezeigte Seite in deinem Browser.".into()),
    }
}

#[tauri::command]
pub async fn open_microsoft_login() -> Result<(), String> {
    // Resolve only the active backend flow. The frontend cannot supply a browser URL.
    let url = state().lock().await.login_url(Instant::now())?.to_owned();
    open_browser_url(&url).await
}

#[tauri::command]
pub async fn open_microsoft_registration() -> Result<(), String> {
    open_browser_url("https://entra.microsoft.com/").await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_at(now: Instant) -> PendingFlow {
        PendingFlow {
            id: "test-flow".into(),
            client_id: "aabbccdd-1122-3344-5566-778899aabbcc".into(),
            device_code: "not-a-real-device-code".into(),
            verification_uri: "https://microsoft.com/devicelogin".into(),
            expires: now + Duration::from_secs(30),
            next_poll: now + Duration::from_secs(5),
            interval: Duration::from_secs(5),
            in_flight: false,
        }
    }

    #[test]
    fn consumer_link_and_eight_character_code_reach_ui_and_browser_unchanged() {
        let now = Instant::now();
        let response = json!({
            "device_code":"synthetic-secret-not-sent-to-ui", "user_code":"ABCD1234",
            "verification_uri":"https://www.microsoft.com/link", "expires_in":900, "interval":5
        });
        let (pending, challenge) = challenge_from_response(
            "aabbccdd-1122-3344-5566-778899aabbcc".into(),
            &response,
            now,
        )
        .unwrap();
        let state = AuthState {
            pending: Some(pending),
            ..AuthState::default()
        };
        let payload = serde_json::to_value(challenge).unwrap();
        assert_eq!(payload["userCode"], "ABCD1234");
        assert_eq!(payload["verificationUri"], "https://www.microsoft.com/link");
        assert_eq!(
            state.login_url(now).unwrap(),
            payload["verificationUri"].as_str().unwrap()
        );
        assert!(payload.get("deviceCode").is_none());
    }

    #[test]
    fn standard_device_login_and_nine_character_code_are_preserved() {
        let now = Instant::now();
        let response = json!({
            "device_code":"synthetic-device-code", "user_code":"ABC-DEFGH",
            "verification_uri":"https://microsoft.com/devicelogin", "expires_in":900, "interval":5
        });
        let (pending, challenge) = challenge_from_response(
            "aabbccdd-1122-3344-5566-778899aabbcc".into(),
            &response,
            now,
        )
        .unwrap();
        let state = AuthState {
            pending: Some(pending),
            ..AuthState::default()
        };
        assert_eq!(challenge.user_code, "ABC-DEFGH");
        assert_eq!(challenge.user_code.len(), 9);
        assert_eq!(
            challenge.verification_uri,
            "https://microsoft.com/devicelogin"
        );
        assert_eq!(state.login_url(now).unwrap(), challenge.verification_uri);
    }

    #[test]
    fn browser_requires_a_live_backend_flow_with_a_verified_url() {
        let now = Instant::now();
        assert!(AuthState::default().login_url(now).is_err());
        let mut state = AuthState {
            pending: Some(pending_at(now)),
            ..AuthState::default()
        };
        assert!(state.login_url(now + Duration::from_secs(30)).is_err());
        state.pending.as_mut().unwrap().verification_uri = "https://example.org/link".into();
        assert!(state.login_url(now).is_err());
        state.pending = None;
        assert!(state.login_url(now).is_err());
    }

    #[test]
    fn polling_obeys_interval_prevents_duplicates_and_stops_at_expiry() {
        let now = Instant::now();
        let mut flow = pending_at(now);
        assert_eq!(flow.reserve_poll(now), PollPermit::Wait);
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(5)),
            PollPermit::Ready
        );
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(10)),
            PollPermit::Wait
        );
        flow.retry_after(now + Duration::from_secs(6), false);
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(10)),
            PollPermit::Wait
        );
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(11)),
            PollPermit::Ready
        );
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(30)),
            PollPermit::Expired
        );
    }

    #[test]
    fn slow_down_extends_wait_for_all_subsequent_requests() {
        let now = Instant::now();
        let mut flow = pending_at(now);
        flow.retry_after(now, true);
        assert_eq!(flow.interval, Duration::from_secs(10));
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(9)),
            PollPermit::Wait
        );
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(10)),
            PollPermit::Ready
        );
        flow.retry_after(now + Duration::from_secs(10), true);
        assert_eq!(flow.interval, Duration::from_secs(15));
        assert_eq!(
            flow.reserve_poll(now + Duration::from_secs(24)),
            PollPermit::Wait
        );
    }

    #[test]
    fn logout_or_replacement_invalidates_in_flight_authentication() {
        let mut state = AuthState {
            generation: 7,
            pending: Some(pending_at(Instant::now())),
            cached: None,
        };
        assert!(state.is_current("test-flow", 7));
        assert!(!state.is_current("different-flow", 7));
        state.generation += 1;
        assert!(!state.is_current("test-flow", 7));
        state.pending = None;
        assert!(!state.is_current("test-flow", 8));
    }

    #[test]
    fn only_own_well_formed_app_ids_are_accepted() {
        assert!(client_id("").is_err());
        assert!(client_id("00000000-0000-0000-0000-000000000000").is_err());
        assert!(client_id("aabbccdd-1122-3344-5566-778899aabbcc").is_ok());
        assert!(client_id("https://example.org").is_err());
    }

    #[test]
    fn device_codes_never_redirect_to_untrusted_sites() {
        assert!(verified_login_url("https://microsoft.com/devicelogin"));
        assert!(verified_login_url(
            "https://login.microsoftonline.com/common/oauth2/deviceauth"
        ));
        for value in [
            "http://microsoft.com/devicelogin",
            "https://microsoft.com.evil.test/devicelogin",
            "https://microsoft.com@evil.test/devicelogin",
            "https://microsoft.com/devicelogin?next=https://evil.test",
            "https://microsoft.com:444/devicelogin",
            "https://microsoft.com/unknown",
        ] {
            assert!(!verified_login_url(value), "{value}");
        }
    }

    #[test]
    fn validates_the_authenticated_minecraft_profile() {
        let profile = json!({"name": "SyncMC_Test", "id": "0123456789ABCDEF0123456789ABCDEF"});
        assert_eq!(
            safe_account(&profile).unwrap().uuid,
            "0123456789abcdef0123456789abcdef"
        );
        assert!(safe_account(
            &json!({"name": "name\n--accessToken", "id": "0123456789abcdef0123456789abcdef"})
        )
        .is_err());
        assert!(safe_account(&json!({"name": "name", "id": "invalid"})).is_err());
    }

    #[test]
    fn requires_a_current_java_license_instead_of_only_a_profile() {
        assert!(check_entitlements(&json!({"items": [{"name":"game_minecraft"}]})).is_ok());
        assert!(check_entitlements(&json!({"items": [{"name":"product_minecraft"}]})).is_ok());
        assert!(check_entitlements(&json!({"items": []})).is_err());
        assert!(check_entitlements(&json!({"items": [{"name":"unrelated_product"}]})).is_err());
        assert!(check_entitlements(&Value::Null).is_err());
    }

    #[test]
    fn service_errors_do_not_expose_provider_body_or_tokens() {
        let body = json!({"access_token": "secret-value", "error_description": "secret-value", "XErr": 2148916238u64});
        assert!(service_error("Xbox", StatusCode::UNAUTHORIZED, &body).contains("Familiengruppe"));
        assert!(!oauth_error(&body).contains("secret-value"));
        assert!(
            !service_error("Minecraft-Anmeldung", StatusCode::FORBIDDEN, &body)
                .contains("secret-value")
        );
        assert!(
            service_error("Minecraft-Anmeldung", StatusCode::FORBIDDEN, &Value::Null)
                .contains("Appregistrierung")
        );
    }

    #[test]
    fn ipc_payloads_contain_only_public_challenge_and_account_fields() {
        let challenge = serde_json::to_value(LoginChallenge {
            flow_id: "flow".into(),
            user_code: "USER-CODE".into(),
            verification_uri: "https://microsoft.com/devicelogin".into(),
            expires_in: 900,
            interval: 5,
        })
        .unwrap();
        assert!(challenge.get("deviceCode").is_none());
        assert!(challenge.get("accessToken").is_none());
        assert_eq!(challenge["flowId"], "flow");
        let account = serde_json::to_value(AccountSummary {
            name: "Player".into(),
            uuid: "0123456789abcdef0123456789abcdef".into(),
        })
        .unwrap();
        assert_eq!(account.as_object().unwrap().len(), 2);
    }
}
