//! Native CurseForge API integration. No API data or reusable download cache is persisted.
//! API: https://docs.curseforge.com/rest-api/
//! CDN authentication: https://blog.curseforge.com/introducing-api-key-authentication-for-curseforge-file-downloads/
//! Dependency policy (EmbeddedLibrary 1 / Include 6): https://docs.curseforge.com/docs/game-integration/unreal/mod-dependencies/
//! Requires the operator's own approved key and permission for this application's use.

use keyring::Entry;
use reqwest::header::{HeaderValue, LOCATION};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Mutex;

const API: &str = "https://api.curseforge.com/v1";
const MINECRAFT: u64 = 432;
const KEY_SERVICE: &str = "dev.syncmc.desktop.curseforge";
const KEY_USER: &str = "approved-api-key-v1";
const APPLICATION_URL: &str = "https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key";
const MAX_METADATA: usize = 8 * 1024 * 1024;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_FILES: usize = 1_000;
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
static KEY_MUTATION: Mutex<()> = Mutex::const_new(());

#[derive(Serialize)]
pub struct CurseForgeStatus {
    configured: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfProject {
    pub id: u64,
    pub game_id: u64,
    pub class_id: Option<u64>,
    pub name: String,
    pub allow_mod_distribution: Option<bool>,
    #[serde(default)]
    pub links: Option<CfLinks>,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub authors: Vec<CfAuthor>,
    pub logo: Option<CfAsset>,
    #[serde(default)]
    pub categories: Vec<CfCategory>,
    #[serde(default)]
    pub latest_files_indexes: Vec<CfFileIndex>,
    #[serde(default)]
    pub screenshots: Vec<CfAsset>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfLinks {
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CfAuthor {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfAsset {
    pub url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfCategory {
    pub id: u64,
    pub game_id: u64,
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub is_class: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfFileIndex {
    pub game_version: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfFile {
    pub id: u64,
    pub mod_id: u64,
    pub game_id: u64,
    pub file_name: String,
    pub file_length: u64,
    pub is_available: bool,
    pub download_url: Option<String>,
    pub game_versions: Vec<String>,
    pub hashes: Vec<CfHash>,
    #[serde(default)]
    pub dependencies: Vec<CfDependency>,
    pub release_type: u8,
    pub file_date: String,
    pub file_status: u8,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub is_server_pack: Option<bool>,
    #[serde(default)]
    pub is_early_access_content: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CfHash {
    pub value: String,
    pub algo: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfDependency {
    pub mod_id: u64,
    pub relation_type: u8,
    #[serde(default)]
    pub file_id: Option<u64>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    index: usize,
    result_count: usize,
    total_count: usize,
}

fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(180))
                // Every authenticated redirect is explicitly checked below.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Der CurseForge-Client konnte nicht gestartet werden.".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn key_header(key: &str) -> Result<HeaderValue, String> {
    if key.is_empty() || key.len() > 512 || key.bytes().any(|b| !b.is_ascii_graphic()) {
        return Err(
            "Der CurseForge-Key ist ungültig. Bitte den eigenen freigegebenen API-Key eingeben."
                .into(),
        );
    }
    let mut header = HeaderValue::from_str(key)
        .map_err(|_| "Der CurseForge-Key hat ein ungültiges Format.".to_owned())?;
    header.set_sensitive(true);
    Ok(header)
}

async fn stored_key() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        let entry = Entry::new(KEY_SERVICE, KEY_USER)
            .map_err(|_| "Der sichere CurseForge-Schlüsselspeicher ist nicht verfügbar.")?;
        match entry.get_password() {
            Ok(key) => {
                key_header(&key)?;
                Ok(Some(key))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Der CurseForge-Key konnte nicht gelesen werden. Bitte den Betriebssystem-Schlüsselbund entsperren.".into()),
        }
    }).await.map_err(|_| "Der sichere CurseForge-Schlüsselspeicher konnte nicht gelesen werden.".to_owned())?
}

async fn required_key() -> Result<String, String> {
    stored_key().await?.ok_or_else(|| "CurseForge ist noch nicht eingerichtet. Dafür wird ein eigener, für SyncMC freigegebener API-Key benötigt.".into())
}

fn http_error(status: StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => "CurseForge hat den API-Key oder den Zugriff abgelehnt. Bitte den eigenen freigegebenen Key und seine Berechtigungen prüfen.".into(),
        404 => "Dieses CurseForge-Projekt oder diese Datei ist über die API nicht verfügbar; möglicherweise ist die Drittanbieter-Verteilung gesperrt.".into(),
        429 => "Zu viele CurseForge-Anfragen. Bitte etwas warten und erneut versuchen.".into(),
        code => format!("CurseForge-Anfrage fehlgeschlagen (HTTP {code})."),
    }
}

async fn response<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, String> {
    let mut response = request.send().await.map_err(|_| {
        "CurseForge ist gerade nicht erreichbar. Bitte die Verbindung prüfen.".to_owned()
    })?;
    if !response.status().is_success() {
        // Do not include provider bodies/URLs: they can contain authentication data.
        return Err(http_error(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA as u64)
    {
        return Err("Die CurseForge-Metadaten sind zu groß.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Die CurseForge-Antwort wurde unterbrochen.".to_owned())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_METADATA {
            return Err("Die CurseForge-Metadaten sind zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "CurseForge hat ungültige Metadaten geliefert.".into())
}

fn request(path: &str, key: &str) -> Result<reqwest::RequestBuilder, String> {
    Ok(client()?
        .get(format!("{API}/{path}"))
        .header("x-api-key", key_header(key)?))
}

fn numeric_id(value: &str) -> Result<u64, String> {
    if value.is_empty()
        || value.len() > 10
        || value.starts_with('0')
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("Die CurseForge-ID ist ungültig.".into());
    }
    let id = value
        .parse::<u64>()
        .map_err(|_| "Die CurseForge-ID ist ungültig.".to_owned())?;
    check_id(id)?;
    Ok(id)
}

fn check_id(id: u64) -> Result<(), String> {
    if id == 0 || id > u32::MAX as u64 {
        return Err("Die CurseForge-ID ist ungültig.".into());
    }
    Ok(())
}

fn bounded(value: &str, maximum: usize, name: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("{name} ist ungültig oder zu lang."));
    }
    Ok(value.into())
}

fn selected(value: &str) -> bool {
    !value.is_empty() && value != "all"
}

fn class_id(kind: &str) -> Result<u64, String> {
    match kind {
        "mod" => Ok(6),
        "resourcepack" => Ok(12),
        "shader" => Ok(6552),
        _ => Err("Dieser CurseForge-Inhaltstyp wird noch nicht unterstützt. Verfügbar sind Mods, Ressourcenpakete und Shader.".into()),
    }
}

fn loader_type(loader: &str) -> Result<Option<u8>, String> {
    match loader {
        "" | "all" => Ok(None),
        "forge" => Ok(Some(1)),
        "fabric" => Ok(Some(4)),
        "quilt" => Ok(Some(5)),
        "neoforge" => Ok(Some(6)),
        _ => Err("Dieser CurseForge-Modloader wird nicht unterstützt.".into()),
    }
}

fn loader_tag(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("Forge"),
        "fabric" => Some("Fabric"),
        "quilt" => Some("Quilt"),
        "neoforge" => Some("NeoForge"),
        _ => None,
    }
}

// Class IDs are verified against the authenticated provider's current category map;
// an unknown/reassigned class fails closed instead of installing into the wrong folder.
async fn verify_class(key: &str, kind: &str) -> Result<(), String> {
    let classes: Envelope<Vec<CfCategory>> = response(request("categories", key)?.query(&[
        ("gameId", MINECRAFT.to_string()),
        ("classesOnly", "true".into()),
    ]))
    .await?;
    validate_class(&classes.data, kind)
}

fn validate_class(classes: &[CfCategory], kind: &str) -> Result<(), String> {
    let id = class_id(kind)?;
    let expected = match kind {
        "mod" => "Mods",
        "resourcepack" => "Resource Packs",
        "shader" => "Shaders",
        _ => unreachable!(),
    };
    if !classes.iter().any(|c| {
        c.id == id && c.game_id == MINECRAFT && c.is_class == Some(true) && c.name == expected
    }) {
        return Err("CurseForge bestätigt die erwartete Minecraft-Inhaltsklasse nicht. Dieser Inhalt kann derzeit nicht sicher zugeordnet werden.".into());
    }
    Ok(())
}

impl CfProject {
    pub(crate) fn kind(&self) -> Result<&'static str, String> {
        match self.class_id {
            Some(6) => Ok("mod"),
            Some(12) => Ok("resourcepack"),
            Some(6552) => Ok("shader"),
            _ => Err("Die CurseForge-Inhaltsklasse wird nicht unterstützt.".into()),
        }
    }

    fn validate(&self, expected: u64) -> Result<(), String> {
        if self.id != expected || self.game_id != MINECRAFT {
            return Err("CurseForge hat ein anderes Projekt oder Spiel geliefert.".into());
        }
        self.kind()?;
        if self.allow_mod_distribution == Some(false) {
            return Err("Der Autor erlaubt für dieses CurseForge-Projekt keine Drittanbieter-Verteilung. SyncMC kann es nicht herunterladen.".into());
        }
        // null means unspecified; the API's own availability/download URL remains authoritative.
        Ok(())
    }
}

fn safe_filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 240
        && !matches!(name, "." | "..")
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        && !(stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit())
}

impl CfFile {
    pub(crate) fn sha1(&self) -> Result<&str, String> {
        let hashes: Vec<_> = self.hashes.iter().filter(|h| h.algo == 1).collect();
        let hash = hashes
            .first()
            .ok_or("Die CurseForge-Datei enthält keinen überprüfbaren SHA1-Hash.")?;
        if hash.value.len() != 40
            || !hash.value.bytes().all(|b| b.is_ascii_hexdigit())
            || hashes
                .iter()
                .any(|h| !h.value.eq_ignore_ascii_case(&hash.value))
        {
            return Err(
                "Die CurseForge-Datei enthält einen ungültigen oder widersprüchlichen SHA1-Hash."
                    .into(),
            );
        }
        Ok(&hash.value)
    }

    fn validate(&self, project_id: u64, file_id: u64) -> Result<(), String> {
        check_id(self.id)?;
        if self.id != file_id || self.mod_id != project_id || self.game_id != MINECRAFT {
            return Err("Die CurseForge-Datei gehört zu einem anderen Projekt oder Spiel.".into());
        }
        if !self.is_available
            || !matches!(self.file_status, 4 | 8 | 10)
            || self.is_server_pack == Some(true)
            || self.is_early_access_content == Some(true)
        {
            return Err(
                "Diese CurseForge-Datei ist nicht als verfügbare Client-Datei freigegeben.".into(),
            );
        }
        if !safe_filename(&self.file_name) || self.file_length == 0 || self.file_length > MAX_FILE {
            return Err("Die CurseForge-Datei hat einen ungültigen Namen oder eine nicht unterstützte Größe.".into());
        }
        self.sha1()?;
        for dependency in &self.dependencies {
            check_id(dependency.mod_id)?;
            if !matches!(dependency.relation_type, 1..=6) {
                return Err("CurseForge hat eine unbekannte Abhängigkeit geliefert.".into());
            }
            if let Some(id) = dependency.file_id.filter(|id| *id != 0) {
                check_id(id)?;
            }
        }
        Ok(())
    }

    pub(crate) fn compatible(&self, kind: &str, game: &str, loader: &str) -> bool {
        if selected(game) && !self.game_versions.iter().any(|v| v == game) {
            return false;
        }
        let lower = self.file_name.to_ascii_lowercase();
        match kind {
            "mod" => {
                lower.ends_with(".jar")
                    && (!selected(loader)
                        || loader_tag(loader)
                            .is_some_and(|tag| self.game_versions.iter().any(|v| v == tag)))
            }
            "resourcepack" | "shader" => lower.ends_with(".zip"),
            _ => false,
        }
    }
}

async fn project_with_key(key: &str, id: u64) -> Result<CfProject, String> {
    check_id(id)?;
    let envelope: Envelope<CfProject> = response(request(&format!("mods/{id}"), key)?).await?;
    envelope.data.validate(id)?;
    verify_class(key, envelope.data.kind()?).await?;
    Ok(envelope.data)
}

async fn file_with_key(key: &str, project_id: u64, file_id: u64) -> Result<CfFile, String> {
    check_id(project_id)?;
    check_id(file_id)?;
    let envelope: Envelope<CfFile> =
        response(request(&format!("mods/{project_id}/files/{file_id}"), key)?).await?;
    envelope.data.validate(project_id, file_id)?;
    Ok(envelope.data)
}

pub(crate) async fn project(id: u64) -> Result<CfProject, String> {
    project_with_key(&required_key().await?, id).await
}

pub(crate) async fn file(project_id: u64, file_id: u64) -> Result<CfFile, String> {
    let key = required_key().await?;
    project_with_key(&key, project_id).await?;
    file_with_key(&key, project_id, file_id).await
}

async fn files_with_key(
    key: &str,
    project: &CfProject,
    game: &str,
    loader: &str,
) -> Result<Vec<CfFile>, String> {
    let kind = project.kind()?;
    let loader_filter = if kind == "mod" {
        loader_type(loader)?
    } else {
        None
    };
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = 0usize;
    loop {
        let mut query = vec![("index", index.to_string()), ("pageSize", "50".into())];
        if selected(game) {
            query.push(("gameVersion", game.into()));
        }
        if let Some(loader) = loader_filter {
            query.push(("modLoaderType", loader.to_string()));
        }
        let page: Envelope<Vec<CfFile>> =
            response(request(&format!("mods/{}/files", project.id), key)?.query(&query)).await?;
        let count = page.data.len();
        if count > 50 {
            return Err("CurseForge hat eine ungültige Dateiseite geliefert.".into());
        }
        for file in page.data {
            if file.mod_id != project.id || file.game_id != MINECRAFT || !seen.insert(file.id) {
                return Err("CurseForge hat falsche oder doppelte Dateieinträge geliefert.".into());
            }
            // Unavailable/history entries are omitted rather than surfaced as installable.
            if file.validate(project.id, file.id).is_ok() && file.compatible(kind, game, loader) {
                files.push(file);
            }
        }
        index += count;
        let more = if let Some(pagination) = page.pagination {
            if pagination.index + pagination.result_count != index
                || pagination.result_count != count
            {
                return Err("CurseForge hat eine ungültige Seitennummerierung geliefert.".into());
            }
            index < pagination.total_count
        } else {
            count == 50
        };
        if !more || count == 0 {
            break;
        }
        if index >= MAX_FILES {
            return Err("Dieses CurseForge-Projekt hat zu viele passende Dateien. Bitte eine konkrete Minecraft-Version auswählen.".into());
        }
    }
    files.sort_by(|a, b| b.file_date.cmp(&a.file_date).then_with(|| b.id.cmp(&a.id)));
    Ok(files)
}

pub(crate) async fn files(
    project_id: u64,
    game_version: &str,
    loader: &str,
) -> Result<Vec<CfFile>, String> {
    let game = bounded(game_version, 64, "Die Minecraft-Version")?;
    let loader = bounded(loader, 32, "Der Modloader")?;
    let key = required_key().await?;
    let project = project_with_key(&key, project_id).await?;
    files_with_key(&key, &project, &game, &loader).await
}

fn download_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value)
        .map_err(|_| "CurseForge hat eine ungültige Download-Adresse geliefert.".to_owned())?;
    // Never synthesize a URL from file IDs or forward a key to arbitrary destinations.
    if value.chars().any(char::is_control)
        || url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("edge.forgecdn.net" | "mediafilez.forgecdn.net" | "media.forgecdn.net")
        )
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/files/")
    {
        return Err("CurseForge hat keine erlaubte offizielle Download-Adresse geliefert.".into());
    }
    Ok(url)
}

async fn supplied_url(key: &str, file: &CfFile) -> Result<Url, String> {
    if let Some(url) = file.download_url.as_deref().filter(|url| !url.is_empty()) {
        return download_url(url);
    }
    // An empty metadata URL is never guessed. Ask the authenticated API once;
    // opt-out was checked on the project before reaching this endpoint.
    let envelope: Envelope<Option<String>> = response(request(
        &format!("mods/{}/files/{}/download-url", file.mod_id, file.id),
        key,
    )?)
    .await?;
    download_url(
        envelope
            .data
            .as_deref()
            .filter(|url| !url.is_empty())
            .ok_or(
                "Der Autor stellt diese CurseForge-Datei nicht für Drittanbieter-Downloads bereit.",
            )?,
    )
}

fn same_artifact(expected: &CfFile, current: &CfFile) -> Result<(), String> {
    if expected.id != current.id
        || expected.mod_id != current.mod_id
        || expected.game_id != current.game_id
        || expected.file_name != current.file_name
        || expected.file_length != current.file_length
        || !expected.sha1()?.eq_ignore_ascii_case(current.sha1()?)
    {
        return Err("Die ausgewählte CurseForge-Datei hat sich geändert. Bitte die Inhaltsauswahl erneut prüfen.".into());
    }
    Ok(())
}

pub(crate) async fn download_file(expected: &CfFile, target: &Path) -> Result<(), String> {
    let key = required_key().await?;
    project_with_key(&key, expected.mod_id).await?;
    let current = file_with_key(&key, expected.mod_id, expected.id).await?;
    same_artifact(expected, &current)?;
    let mut url = supplied_url(&key, &current).await?;
    let mut final_response = None;
    for redirects in 0..=4 {
        let result = client()?
            .get(url.clone())
            .header("x-api-key", key_header(&key)?)
            .send()
            .await
            .map_err(|_| "Der CurseForge-Download wurde unterbrochen.".to_owned())?;
        if result.status().is_redirection() {
            if redirects == 4 {
                return Err("Der CurseForge-Download wurde zu oft weitergeleitet.".into());
            }
            let location = result
                .headers()
                .get(LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or("CurseForge hat eine ungültige Weiterleitung geliefert.")?;
            let next = url
                .join(location)
                .map_err(|_| "Die CurseForge-Weiterleitung ist ungültig.".to_owned())?;
            url = download_url(next.as_str())?;
            continue;
        }
        if !result.status().is_success() {
            return Err(http_error(result.status()));
        }
        final_response = Some(result);
        break;
    }
    let mut response = final_response.ok_or("CurseForge hat keine Datei geliefert.")?;
    if response
        .content_length()
        .is_some_and(|size| size != current.file_length)
    {
        return Err("Die CurseForge-Downloadgröße stimmt nicht mit den Metadaten überein.".into());
    }
    let parent = target
        .parent()
        .ok_or("Das CurseForge-Downloadziel ist ungültig.")?;
    if fs::symlink_metadata(parent)
        .map_err(|_| "Der Inhaltsordner ist nicht verfügbar.")?
        .file_type()
        .is_symlink()
    {
        return Err("Der Inhaltsordner darf keine symbolische Verknüpfung sein.".into());
    }
    if target.symlink_metadata().is_ok() {
        return Err("Das CurseForge-Downloadziel existiert bereits.".into());
    }
    let temp = parent.join(format!(".syncmc-cf-{}.part", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&temp)
        .map_err(|_| "Die temporäre CurseForge-Datei konnte nicht angelegt werden.")?;
    let result: Result<(), String> = async {
        let mut size = 0u64;
        let mut digest = Sha1::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Der CurseForge-Download wurde unterbrochen.")?
        {
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or("Die CurseForge-Datei ist zu groß.")?;
            if size > current.file_length || size > MAX_FILE {
                return Err("Die CurseForge-Datei ist größer als angekündigt.".into());
            }
            digest.update(&chunk);
            output
                .write_all(&chunk)
                .map_err(|_| "Die CurseForge-Datei konnte nicht geschrieben werden.")?;
        }
        let hash = format!("{:x}", digest.finalize());
        if size != current.file_length || !hash.eq_ignore_ascii_case(current.sha1()?) {
            return Err("Die CurseForge-Datei hat die SHA1-/Größenprüfung nicht bestanden.".into());
        }
        output
            .sync_all()
            .map_err(|_| "Die CurseForge-Datei konnte nicht gespeichert werden.")?;
        // create_new reserves the exact destination so a pre-existing file is never overwritten.
        let reservation = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|_| "Das CurseForge-Downloadziel wurde inzwischen belegt.")?;
        drop(reservation);
        if let Err(_) = fs::rename(&temp, target) {
            let _ = fs::remove_file(target);
            return Err("Die geprüfte CurseForge-Datei konnte nicht übernommen werden.".into());
        }
        Ok(())
    }
    .await;
    drop(output);
    let _ = fs::remove_file(&temp);
    result
}

fn safe_image(value: &str) -> String {
    Url::parse(value)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some("media.forgecdn.net" | "mediafilez.forgecdn.net")
                )
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.port_or_known_default() == Some(443)
        })
        .map(|url| url.to_string())
        .unwrap_or_default()
}

fn safe_source_url(value: &str) -> Option<String> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn normalized_hit(project: &CfProject) -> Result<Value, String> {
    project.validate(project.id)?;
    let kind = project.kind()?;
    let versions: BTreeSet<_> = project
        .latest_files_indexes
        .iter()
        .map(|f| &f.game_version)
        .collect();
    Ok(json!({
        "project_id":project.id.to_string(), "slug":project.slug, "title":project.name,
        "description":project.summary, "source":"curseforge", "project_type":kind,
        "source_url":project.links.as_ref().and_then(|links|links.source_url.as_deref()).and_then(safe_source_url),
        "icon_url":project.logo.as_ref().map(|a| safe_image(&a.url)).unwrap_or_default(),
        "author":project.authors.iter().map(|a|a.name.as_str()).collect::<Vec<_>>().join(", "),
        "downloads":project.download_count, "categories":project.categories.iter().map(|c|&c.slug).collect::<Vec<_>>(),
        "versions":versions, "gallery":project.screenshots.iter().map(|a|safe_image(&a.url)).filter(|s|!s.is_empty()).collect::<Vec<_>>()
    }))
}

fn normalized_version(file: &CfFile, kind: &str) -> Result<Value, String> {
    file.validate(file.mod_id, file.id)?;
    let dependencies = file.dependencies.iter().map(|d| {
        // Include is bundled; EmbeddedLibrary is a separate library in CF's official policy.
        let relation = match d.relation_type { 1 | 3 => "required", 2 | 4 => "optional", 5 => "incompatible", 6 => "embedded", _ => "optional" };
        json!({"project_id":d.mod_id.to_string(), "version_id":d.file_id.filter(|id|*id!=0).map(|id|id.to_string()), "dependency_type":relation})
    }).collect::<Vec<_>>();
    let loaders = if kind == "mod" {
        ["fabric", "forge", "quilt", "neoforge"]
            .into_iter()
            .filter(|l| {
                loader_tag(l).is_some_and(|tag| file.game_versions.iter().any(|v| v == tag))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(json!({
        "id":file.id.to_string(), "project_id":file.mod_id.to_string(), "source":"curseforge", "project_type":kind,
        "name":file.display_name, "version_number":if file.display_name.is_empty(){&file.file_name}else{&file.display_name},
        "version_type":match file.release_type{2=>"beta",3=>"alpha",_=>"release"},
        "date_published":file.file_date, "game_versions":file.game_versions, "loaders":loaders,
        // IPC carries no authenticated URL; the installer resolves URLs afresh in Rust.
        "files":[{"filename":file.file_name,"url":"","primary":true,"size":file.file_length,"hashes":{"sha1":file.sha1()?}}],
        "dependencies":dependencies
    }))
}

#[tauri::command]
pub async fn curseforge_status() -> Result<CurseForgeStatus, String> {
    Ok(CurseForgeStatus {
        configured: stored_key().await?.is_some(),
    })
}

#[tauri::command]
pub async fn set_curseforge_key(key: String) -> Result<CurseForgeStatus, String> {
    // No trimming/echoing: a pasted malformed key fails without revealing its contents.
    key_header(&key)?;
    let _guard = KEY_MUTATION.lock().await;
    let game: Envelope<Value> = response(request(&format!("games/{MINECRAFT}"), &key)?).await?;
    if game.data.get("id").and_then(Value::as_u64) != Some(MINECRAFT)
        || game.data.get("slug").and_then(Value::as_str) != Some("minecraft")
    {
        return Err("Der CurseForge-Key konnte für Minecraft nicht validiert werden.".into());
    }
    tokio::task::spawn_blocking(move || {
        Entry::new(KEY_SERVICE, KEY_USER)
            .map_err(|_| "Der sichere CurseForge-Schlüsselspeicher ist nicht verfügbar.")?
            .set_password(&key)
            .map_err(|_| "Der CurseForge-Key konnte nicht sicher gespeichert werden. Bitte den Betriebssystem-Schlüsselbund entsperren; es wird keine Ersatzdatei angelegt.".to_owned())
    }).await.map_err(|_| "Der CurseForge-Key konnte nicht sicher gespeichert werden.".to_owned())??;
    Ok(CurseForgeStatus { configured: true })
}

#[tauri::command]
pub async fn clear_curseforge_key() -> Result<CurseForgeStatus, String> {
    let _guard = KEY_MUTATION.lock().await;
    tokio::task::spawn_blocking(|| {
        let entry = Entry::new(KEY_SERVICE, KEY_USER)
            .map_err(|_| "Der sichere CurseForge-Schlüsselspeicher ist nicht verfügbar.")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Der CurseForge-Key konnte nicht entfernt werden. Bitte den Betriebssystem-Schlüsselbund entsperren.".to_owned()),
        }
    }).await.map_err(|_| "Der CurseForge-Key konnte nicht entfernt werden.".to_owned())??;
    Ok(CurseForgeStatus { configured: false })
}

#[tauri::command]
pub async fn search_curseforge(
    query: String,
    game_version: String,
    loader: String,
    project_type: String,
) -> Result<Value, String> {
    let query = bounded(&query, 200, "Die Suchanfrage")?;
    let game = bounded(&game_version, 64, "Die Minecraft-Version")?;
    let loader = bounded(&loader, 32, "Der Modloader")?;
    let class = class_id(&project_type)?;
    let filter = if project_type == "mod" {
        loader_type(&loader)?
    } else {
        None
    };
    if filter.is_some() && !selected(&game) {
        return Err(
            "Bitte für die CurseForge-Modloader-Suche eine Minecraft-Version auswählen.".into(),
        );
    }
    let key = required_key().await?;
    verify_class(&key, &project_type).await?;
    let mut parameters = vec![
        ("gameId", MINECRAFT.to_string()),
        ("classId", class.to_string()),
        ("searchFilter", query),
        ("pageSize", "24".into()),
        ("sortField", "2".into()),
        ("sortOrder", "desc".into()),
    ];
    if selected(&game) {
        parameters.push(("gameVersion", game));
    }
    if let Some(loader) = filter {
        parameters.push(("modLoaderType", loader.to_string()));
    }
    let data: Envelope<Vec<CfProject>> =
        response(request("mods/search", &key)?.query(&parameters)).await?;
    if data.data.len() > 24 {
        return Err("CurseForge hat zu viele Suchtreffer geliefert.".into());
    }
    let mut hits = Vec::new();
    for project in data.data {
        if project.game_id != MINECRAFT || project.class_id != Some(class) {
            return Err("CurseForge hat Treffer für eine andere Inhaltsklasse geliefert.".into());
        }
        if project.allow_mod_distribution == Some(false) {
            continue;
        }
        hits.push(normalized_hit(&project)?);
    }
    Ok(json!({"hits":hits}))
}

#[tauri::command]
pub async fn get_curseforge_versions(
    project_id: String,
    game_version: String,
    loader: String,
    project_type: Option<String>,
) -> Result<Value, String> {
    let id = numeric_id(&project_id)?;
    let game = bounded(&game_version, 64, "Die Minecraft-Version")?;
    let loader = bounded(&loader, 32, "Der Modloader")?;
    let key = required_key().await?;
    let project = project_with_key(&key, id).await?;
    let kind = project.kind()?;
    if project_type
        .as_deref()
        .is_some_and(|kind| class_id(kind).ok() != project.class_id)
    {
        return Err("Das CurseForge-Projekt gehört zu einer anderen Inhaltsklasse.".into());
    }
    let versions = files_with_key(&key, &project, &game, &loader).await?;
    let mut result = Vec::new();
    for file in versions {
        // Check download permission without inventing CDN addresses or returning them to IPC.
        if file
            .download_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
        {
            download_url(file.download_url.as_deref().unwrap())?;
        }
        result.push(normalized_version(&file, kind)?);
    }
    Ok(Value::Array(result))
}

#[tauri::command]
pub async fn open_curseforge_application() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = tokio::process::Command::new("open");
        c.arg(APPLICATION_URL);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = tokio::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", APPLICATION_URL]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut c = tokio::process::Command::new("xdg-open");
        c.arg(APPLICATION_URL);
        c
    };
    match tokio::time::timeout(Duration::from_secs(10), command.kill_on_drop(true)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => Err("Der Systembrowser konnte nicht geöffnet werden. Bitte die offizielle CurseForge-API-Hilfeseite im Browser öffnen.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CfFile {
        serde_json::from_value(json!({"id":123456,"modId":789,"gameId":432,"fileName":"example.jar",
            "fileLength":3,"isAvailable":true,"downloadUrl":"https://edge.forgecdn.net/files/123/456/example.jar",
            "gameVersions":["1.21.1","Fabric"],"hashes":[{"algo":2,"value":"md5-unused"},{"algo":1,"value":"a9993e364706816aba3e25717850c26c9cd0d89d"}],
            "dependencies":[{"modId":456,"relationType":3}],"releaseType":1,"fileDate":"2026-11-05T12:00:00Z","fileStatus":4})).unwrap()
    }

    fn fixture_project() -> CfProject {
        serde_json::from_value(
            json!({"id":789,"gameId":432,"classId":6,"name":"Example","allowModDistribution":true}),
        )
        .unwrap()
    }

    #[test]
    fn search_source_repository_hint_uses_existing_project_links_without_extra_requests() {
        let project:CfProject=serde_json::from_value(json!({"id":789,"gameId":432,"classId":6,"name":"Example","allowModDistribution":true,"links":{"sourceUrl":"https://github.com/Owner/Repo?token=synthetic-secret#readme","other":"synthetic-secret"}})).unwrap();
        let hit = normalized_hit(&project).unwrap();
        assert_eq!(hit["source_url"], "https://github.com/Owner/Repo");
        assert!(!hit.to_string().contains("synthetic-secret"));
        let empty:CfProject=serde_json::from_value(json!({"id":789,"gameId":432,"classId":6,"name":"Example","allowModDistribution":true,"links":{"sourceUrl":null}})).unwrap();
        assert!(normalized_hit(&empty).unwrap()["source_url"].is_null());
        assert!(normalized_hit(&fixture_project()).unwrap()["source_url"].is_null());
    }

    #[test]
    fn source_repository_hint_rejects_credentialed_and_unsafe_urls() {
        for url in [
            "http://github.com/Owner/Repo",
            "https://name:secret@github.com/Owner/Repo",
            "https://github.com:444/Owner/Repo",
            "file:///tmp/repo",
            "https://github.com/Owner/Repo\n",
        ] {
            assert!(safe_source_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn identifiers_and_key_errors_never_echo_input() {
        for value in ["0", "01", "-1", "1/2", "4294967296", "https://example.org"] {
            assert!(numeric_id(value).is_err());
        }
        assert_eq!(numeric_id("123456").unwrap(), 123456);
        let secret = "synthetic-secret\r\nInjected:bad";
        assert!(!key_header(secret).unwrap_err().contains("synthetic-secret"));
        let header = key_header("synthetic-valid-key").unwrap();
        assert!(header.is_sensitive());
        for code in [401, 403, 404, 429, 500] {
            assert!(!http_error(StatusCode::from_u16(code).unwrap()).contains("synthetic"));
        }
    }

    #[test]
    fn minecraft_classes_are_verified_and_unknown_content_is_rejected() {
        let categories: Vec<CfCategory> = serde_json::from_value(json!([
            {"id":6,"gameId":432,"name":"Mods","isClass":true},
            {"id":12,"gameId":432,"name":"Resource Packs","isClass":true},
            {"id":6552,"gameId":432,"name":"Shaders","isClass":true}]))
        .unwrap();
        for kind in ["mod", "resourcepack", "shader"] {
            validate_class(&categories, kind).unwrap();
        }
        assert!(class_id("datapack").is_err());
        assert!(class_id("modpack").is_err());
        assert!(validate_class(&categories[..1], "shader").is_err());
        let mut project = fixture_project();
        project.game_id = 1;
        assert!(project.validate(789).is_err());
    }

    #[test]
    fn nullable_category_class_marker_does_not_break_project_search_metadata() {
        let project: CfProject = serde_json::from_value(json!({
            "id":789,"gameId":432,"classId":6,"name":"Example","allowModDistribution":true,
            "categories":[{"id":423,"gameId":432,"name":"Map and Information","isClass":null}]
        }))
        .unwrap();
        assert!(project.categories[0].is_class.is_none());
        assert_eq!(normalized_hit(&project).unwrap()["project_type"], "mod");

        // A null class marker can be read but cannot authorize a folder mapping.
        let classes: Vec<CfCategory> = serde_json::from_value(json!([
            {"id":6,"gameId":432,"name":"Mods","isClass":null}
        ]))
        .unwrap();
        assert!(validate_class(&classes, "mod").is_err());
    }

    #[test]
    fn respects_distribution_and_unavailable_server_malware_files() {
        let mut project = fixture_project();
        project.allow_mod_distribution = Some(false);
        assert!(project.validate(789).unwrap_err().contains("Drittanbieter"));
        project.allow_mod_distribution = None;
        assert!(project.validate(789).is_ok());
        let mut file = fixture();
        file.validate(789, 123456).unwrap();
        file.is_available = false;
        assert!(file.validate(789, 123456).is_err());
        file.is_available = true;
        file.is_server_pack = Some(true);
        assert!(file.validate(789, 123456).is_err());
        file.is_server_pack = None;
        file.file_status = 6;
        assert!(file.validate(789, 123456).is_err());
        file.file_status = 4;
        file.game_id = 1;
        assert!(file.validate(789, 123456).is_err());
    }

    #[test]
    fn archived_files_remain_usable_only_while_authorized_and_available() {
        // Official support: archived files may still be downloaded as dependencies.
        // https://support.curseforge.com/support/solutions/articles/9000197242
        let mut file = fixture();
        file.file_status = 8;
        file.validate(789, 123456).unwrap();
        file.is_available = false;
        assert!(file.validate(789, 123456).is_err());
        file.is_available = true;
        for blocked in [5, 6, 7] {
            file.file_status = blocked;
            assert!(file.validate(789, 123456).is_err());
        }
    }

    #[test]
    fn exact_compatibility_not_project_or_filename_guessing() {
        let file = fixture();
        assert!(file.compatible("mod", "1.21.1", "fabric"));
        assert!(!file.compatible("mod", "1.21", "fabric"));
        assert!(!file.compatible("mod", "1.21.1", "forge"));
        assert!(!file.compatible("shader", "1.21.1", "fabric"));
        let mut resource = file.clone();
        resource.file_name = "pack.zip".into();
        resource.game_versions = vec!["1.21.1".into()];
        assert!(resource.compatible("resourcepack", "1.21.1", "fabric"));
        assert!(resource.compatible("shader", "1.21.1", "fabric"));
        assert_eq!(loader_type("fabric").unwrap(), Some(4));
        assert!(loader_type("guessed").is_err());
    }

    #[test]
    fn sha1_is_selected_by_algorithm_and_artifact_changes_are_rejected() {
        let file = fixture();
        assert_eq!(
            file.sha1().unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let mut altered = file.clone();
        altered.hashes.retain(|h| h.algo == 2);
        assert!(altered.sha1().is_err());
        altered = file.clone();
        altered.hashes.push(CfHash {
            value: "0000000000000000000000000000000000000000".into(),
            algo: 1,
        });
        assert!(altered.sha1().is_err());
        altered = file.clone();
        altered.file_name = "other.jar".into();
        assert!(same_artifact(&file, &altered).is_err());
        for name in [
            "../x.jar", "x\\x.jar", "CON.jar", "LPT1.zip", "x.jar.", "a:b.jar",
        ] {
            assert!(!safe_filename(name));
        }
    }

    #[test]
    fn download_header_is_only_sent_to_exact_official_hosts() {
        for valid in [
            "https://edge.forgecdn.net/files/1/2/a.jar",
            "https://mediafilez.forgecdn.net/files/1/2/a.jar",
            "https://media.forgecdn.net/files/1/2/a.jar",
        ] {
            assert!(download_url(valid).is_ok());
        }
        for invalid in [
            "http://edge.forgecdn.net/files/a.jar",
            "https://edge.forgecdn.net.evil.org/files/a.jar",
            "https://forgecdn.net/files/a.jar",
            "https://user@edge.forgecdn.net/files/a.jar",
            "https://edge.forgecdn.net:444/files/a.jar",
            "https://edge.forgecdn.net/files/a.jar?api-key=secret",
            "https://edge.forgecdn.net/notfiles/a.jar",
            "https://example.org/files/a.jar",
        ] {
            assert!(download_url(invalid).is_err());
        }
        let base = download_url("https://edge.forgecdn.net/files/1/2/a.jar").unwrap();
        assert!(download_url(
            base.join("https://example.org/files/a.jar")
                .unwrap()
                .as_str()
        )
        .is_err());
    }

    #[test]
    fn normalized_schema_preserves_provider_and_sha1_without_download_secrets() {
        let mut file = fixture();
        file.dependencies = (1..=6)
            .map(|relation_type| CfDependency {
                mod_id: 100 + relation_type as u64,
                relation_type,
                file_id: None,
            })
            .collect();
        let normalized = normalized_version(&file, "mod").unwrap();
        assert_eq!(normalized["source"], "curseforge");
        assert_eq!(normalized["project_type"], "mod");
        assert_eq!(normalized["project_id"], "789");
        assert_eq!(normalized["files"][0]["url"], "");
        assert!(normalized["files"][0]["hashes"].get("sha512").is_none());
        assert_eq!(normalized["dependencies"][0]["dependency_type"], "required");
        assert_eq!(normalized["dependencies"][5]["dependency_type"], "embedded");
        assert!(normalized_hit(&fixture_project())
            .unwrap()
            .get("source")
            .is_some());
        assert!(!normalized.to_string().contains("forgecdn"));
    }
}
