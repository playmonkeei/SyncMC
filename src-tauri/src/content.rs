//! Pinned provider artifacts, required dependencies, and local installation proof.
//!
//! Metadata is always fetched from Modrinth; imported profiles never supply download URLs.
//! API schema: https://docs.modrinth.com/api/operations/getversion/

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

const API: &str = "https://api.modrinth.com/v2";
const MAX_CONTENTS: usize = 1_000;
const MAX_ARTIFACT_SIZE: u64 = 512 * 1024 * 1024;
const MAX_METADATA_SIZE: usize = 8 * 1024 * 1024;
pub static CONTENT_LOCK: Mutex<()> = Mutex::const_new(());
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSelection {
    pub id: String,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version: String,
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub kind: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    pub source: String,
    pub status: String,
    /// Automatically resolved entries remain pinned, but cease to be roots when removed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dependency: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReport {
    pub contents: Vec<ContentSelection>,
    pub files_verified: usize,
    pub dependency_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<crate::backup::BackupSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentProgress {
    pub instance_id: String,
    pub completed_files: usize,
    pub total_files: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDependency {
    version_id: Option<String>,
    project_id: Option<String>,
    dependency_type: String,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiFile {
    filename: String,
    url: String,
    hashes: BTreeMap<String, String>,
    primary: bool,
    size: u64,
    #[serde(default)]
    file_type: Option<String>,
    #[serde(default = "modrinth_source")]
    source: String,
    #[serde(skip)]
    curseforge_file: Option<crate::curseforge::CfFile>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiVersion {
    #[serde(default = "modrinth_source")]
    source: String,
    id: String,
    project_id: String,
    version_number: String,
    version_type: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    files: Vec<ApiFile>,
    #[serde(default)]
    dependencies: Vec<ApiDependency>,
    #[serde(default)]
    date_published: String,
    /// Modrinth's newer environment enum supersedes project-wide client_side.
    #[serde(default)]
    environment: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiProject {
    id: String,
    title: String,
    project_type: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    client_side: Option<String>,
    #[serde(default)]
    environment: Vec<String>,
    #[serde(default = "distribution_allowed")]
    distribution_allowed: bool,
}

fn modrinth_source() -> String {
    "modrinth".into()
}
fn distribution_allowed() -> bool {
    true
}
fn project_key(source: &str, project: &str) -> String {
    format!("{source}:{project}")
}

fn provider_identifier(source: &str, value: &str, label: &str) -> Result<(), String> {
    identifier(value, label)?;
    match source {
        "modrinth" => Ok(()),
        "curseforge" => match value.parse::<u32>() {
            Ok(number) if number > 0 && number.to_string() == value => Ok(()),
            _ => Err(format!(
                "{label} muss eine positive CurseForge-ID ohne führende Nullen sein."
            )),
        },
        _ => Err("Diese Inhaltsquelle wird nicht unterstützt.".into()),
    }
}

trait Metadata {
    fn version(&mut self, id: &str) -> impl Future<Output = Result<ApiVersion, String>> + Send;
    fn project(&mut self, id: &str) -> impl Future<Output = Result<ApiProject, String>> + Send;
    fn versions(
        &mut self,
        project: &str,
        game: &str,
    ) -> impl Future<Output = Result<Vec<ApiVersion>, String>> + Send;
}

trait SourcesMetadata {
    fn version_for(
        &mut self,
        source: &str,
        project: Option<&str>,
        id: &str,
    ) -> impl Future<Output = Result<ApiVersion, String>> + Send;
    fn project_for(
        &mut self,
        source: &str,
        id: &str,
    ) -> impl Future<Output = Result<ApiProject, String>> + Send;
    fn versions_for(
        &mut self,
        source: &str,
        project: &str,
        game: &str,
        loader: &str,
    ) -> impl Future<Output = Result<Vec<ApiVersion>, String>> + Send;
}

impl<R: Metadata + Send> SourcesMetadata for R {
    async fn version_for(
        &mut self,
        source: &str,
        _project: Option<&str>,
        id: &str,
    ) -> Result<ApiVersion, String> {
        if source != "modrinth" {
            return Err("Diese Test-Metadatenquelle unterstützt nur Modrinth.".into());
        }
        self.version(id).await
    }
    async fn project_for(&mut self, source: &str, id: &str) -> Result<ApiProject, String> {
        if source != "modrinth" {
            return Err("Diese Test-Metadatenquelle unterstützt nur Modrinth.".into());
        }
        self.project(id).await
    }
    async fn versions_for(
        &mut self,
        source: &str,
        project: &str,
        game: &str,
        _loader: &str,
    ) -> Result<Vec<ApiVersion>, String> {
        if source != "modrinth" {
            return Err("Diese Test-Metadatenquelle unterstützt nur Modrinth.".into());
        }
        self.versions(project, game).await
    }
}

#[derive(Default)]
struct ProvidersMetadata {
    modrinth: ModrinthMetadata,
    projects: BTreeMap<String, ApiProject>,
    versions: BTreeMap<String, ApiVersion>,
}

fn curseforge_project(project: &crate::curseforge::CfProject) -> Result<ApiProject, String> {
    if project.game_id != 432 {
        return Err("Das CurseForge-Projekt gehört nicht zu Minecraft.".into());
    }
    Ok(ApiProject {
        id: project.id.to_string(),
        title: project.name.clone(),
        project_type: project.kind()?.into(),
        icon_url: None,
        client_side: None,
        environment: Vec::new(),
        distribution_allowed: project.allow_mod_distribution != Some(false),
    })
}

fn curseforge_version(file: crate::curseforge::CfFile) -> Result<ApiVersion, String> {
    provider_identifier("curseforge", &file.id.to_string(), "Datei-ID")?;
    provider_identifier("curseforge", &file.mod_id.to_string(), "Projekt-ID")?;
    if file.game_id != 432
        || !file.is_available
        || !matches!(file.file_status, 4 | 8 | 10)
        || file.is_server_pack == Some(true)
        || file.is_early_access_content == Some(true)
    {
        return Err(
            "Diese CurseForge-Datei steht nicht als verfügbare Client-Datei für SyncMC bereit."
                .into(),
        );
    }
    let hash = file.sha1()?.to_ascii_lowercase();
    let dependencies = file
        .dependencies
        .iter()
        .filter_map(|dependency| {
            let dependency_type = match dependency.relation_type {
                1 | 3 => "required",
                2 | 4 => "optional",
                5 => "incompatible",
                6 => "embedded",
                _ => {
                    return Some(Err(
                        "Eine CurseForge-Abhängigkeit hat einen unbekannten Beziehungstyp.".into(),
                    ))
                }
            };
            Some(Ok(ApiDependency {
                project_id: Some(dependency.mod_id.to_string()),
                version_id: dependency
                    .file_id
                    .filter(|id| *id > 0)
                    .map(|id| id.to_string()),
                dependency_type: dependency_type.into(),
                file_name: None,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let loaders = file
        .game_versions
        .iter()
        .filter_map(|value| match value.to_ascii_lowercase().as_str() {
            "fabric" => Some("fabric".into()),
            "forge" => Some("forge".into()),
            "neoforge" => Some("neoforge".into()),
            "quilt" => Some("quilt".into()),
            _ => None,
        })
        .collect();
    Ok(ApiVersion {
        source: "curseforge".into(),
        id: file.id.to_string(),
        project_id: file.mod_id.to_string(),
        version_number: file.file_name.clone(),
        version_type: match file.release_type {
            1 => "release",
            2 => "beta",
            _ => "alpha",
        }
        .into(),
        game_versions: file.game_versions.clone(),
        loaders,
        files: vec![ApiFile {
            filename: file.file_name.clone(),
            url: file.download_url.clone().unwrap_or_default(),
            hashes: BTreeMap::from([("sha1".into(), hash)]),
            primary: true,
            size: file.file_length,
            file_type: None,
            source: "curseforge".into(),
            curseforge_file: Some(file.clone()),
        }],
        dependencies,
        date_published: file.file_date,
        environment: None,
    })
}

impl SourcesMetadata for ProvidersMetadata {
    async fn project_for(&mut self, source: &str, id: &str) -> Result<ApiProject, String> {
        provider_identifier(source, id, "Projekt-ID")?;
        if source == "modrinth" {
            return self.modrinth.project(id).await;
        }
        let key = project_key(source, id);
        if let Some(project) = self.projects.get(&key) {
            return Ok(project.clone());
        }
        let project = curseforge_project(
            &crate::curseforge::project(id.parse().map_err(|_| "Ungültige CurseForge-ID.")?)
                .await?,
        )?;
        if !project.distribution_allowed {
            return Err("Der Autor erlaubt den Download dieses CurseForge-Projekts durch andere Launcher nicht.".into());
        }
        self.projects.insert(key, project.clone());
        Ok(project)
    }
    async fn version_for(
        &mut self,
        source: &str,
        project: Option<&str>,
        id: &str,
    ) -> Result<ApiVersion, String> {
        provider_identifier(source, id, "Versions-ID")?;
        if source == "modrinth" {
            return self.modrinth.version(id).await;
        }
        let project = project.ok_or("Für eine CurseForge-Datei fehlt die Projekt-ID.")?;
        provider_identifier(source, project, "Projekt-ID")?;
        self.project_for(source, project).await?;
        let key = project_key(source, id);
        if let Some(version) = self.versions.get(&key) {
            if version.project_id != project {
                return Err("Die CurseForge-Datei gehört zu einem anderen Projekt.".into());
            }
            return Ok(version.clone());
        }
        let version = curseforge_version(
            crate::curseforge::file(
                project
                    .parse()
                    .map_err(|_| "Ungültige CurseForge-Projekt-ID.")?,
                id.parse().map_err(|_| "Ungültige CurseForge-Datei-ID.")?,
            )
            .await?,
        )?;
        self.versions.insert(key, version.clone());
        Ok(version)
    }
    async fn versions_for(
        &mut self,
        source: &str,
        project: &str,
        game: &str,
        loader: &str,
    ) -> Result<Vec<ApiVersion>, String> {
        provider_identifier(source, project, "Projekt-ID")?;
        if source == "modrinth" {
            return self.modrinth.versions(project, game).await;
        }
        self.project_for(source, project).await?;
        let versions = crate::curseforge::files(
            project
                .parse()
                .map_err(|_| "Ungültige CurseForge-Projekt-ID.")?,
            game,
            loader,
        )
        .await?
        .into_iter()
        .map(curseforge_version)
        .collect::<Result<Vec<_>, _>>()?;
        for version in &versions {
            self.versions
                .insert(project_key(source, &version.id), version.clone());
        }
        Ok(versions)
    }
}

#[derive(Default)]
struct ModrinthMetadata {
    versions: BTreeMap<String, ApiVersion>,
    projects: BTreeMap<String, ApiProject>,
}

fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(concat!(
                    "SyncMC/",
                    env!("CARGO_PKG_VERSION"),
                    " (desktop Minecraft launcher)"
                ))
                .https_only(true)
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Der Modrinth-Downloadclient konnte nicht gestartet werden.".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

async fn metadata<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, String> {
    let mut response = request
        .send()
        .await
        .map_err(|_| "Modrinth ist nicht erreichbar. Bitte die Verbindung prüfen.")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            404 => "Eine vorgemerkte Modrinth-Version oder ein Projekt ist nicht mehr verfügbar."
                .into(),
            429 => "Zu viele Modrinth-Anfragen. Bitte etwas warten und erneut versuchen.".into(),
            status => format!("Modrinth-Anfrage fehlgeschlagen (HTTP {status})."),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Die Modrinth-Antwort wurde unterbrochen.")?
    {
        if bytes.len() + chunk.len() > MAX_METADATA_SIZE {
            return Err("Die Modrinth-Metadaten sind zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Modrinth hat ungültige Versionsdaten geliefert.".into())
}

impl Metadata for ModrinthMetadata {
    async fn version(&mut self, id: &str) -> Result<ApiVersion, String> {
        identifier(id, "Modrinth-Versions-ID")?;
        if let Some(version) = self.versions.get(id) {
            return Ok(version.clone());
        }
        let version: ApiVersion = metadata(client()?.get(format!("{API}/version/{id}"))).await?;
        if version.id != id {
            return Err("Modrinth hat eine andere als die ausgewählte Version geliefert.".into());
        }
        self.versions.insert(id.into(), version.clone());
        Ok(version)
    }

    async fn project(&mut self, id: &str) -> Result<ApiProject, String> {
        identifier(id, "Modrinth-Projekt-ID")?;
        if let Some(project) = self.projects.get(id) {
            return Ok(project.clone());
        }
        let project: ApiProject = metadata(client()?.get(format!("{API}/project/{id}"))).await?;
        if project.id != id {
            return Err("Modrinth hat ein unerwartetes Projekt geliefert.".into());
        }
        self.projects.insert(id.into(), project.clone());
        Ok(project)
    }

    async fn versions(&mut self, project: &str, game: &str) -> Result<Vec<ApiVersion>, String> {
        identifier(project, "Modrinth-Projekt-ID")?;
        let versions: Vec<ApiVersion> = metadata(
            client()?
                .get(format!("{API}/project/{project}/version"))
                .query(&[
                    ("game_versions", serde_json::json!([game]).to_string()),
                    ("include_changelog", "false".into()),
                ]),
        )
        .await?;
        for version in &versions {
            if version.project_id != project {
                return Err("Eine Modrinth-Version gehört zum falschen Projekt.".into());
            }
            self.versions.insert(version.id.clone(), version.clone());
        }
        Ok(versions)
    }
}

fn identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("{label} ist ungültig."));
    }
    Ok(())
}

fn filename(value: &str) -> Result<(), String> {
    let stem = value.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if value.is_empty()
        || value.len() > 240
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || reserved
    {
        return Err("Ein Inhaltsdateiname ist für macOS und Windows ungültig.".into());
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 128 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn directory(kind: &str) -> Result<&'static str, String> {
    match kind {
        "mod" => Ok("mods"),
        "shader" => Ok("shaderpacks"),
        "resourcepack" => Ok("resourcepacks"),
        "datapack" => Err("Data Packs gehören in eine konkrete Welt. Entferne oder deaktiviere sie für die Client-Installation.".into()),
        _ => Err("Dieser Inhaltstyp kann nicht installiert werden.".into()),
    }
}

pub(crate) fn validate_selections(contents: &[ContentSelection]) -> Result<(), String> {
    if contents.len() > MAX_CONTENTS {
        return Err("Eine Instanz darf höchstens 1.000 Inhalte enthalten.".into());
    }
    let mut projects = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut filenames = BTreeSet::new();
    for item in contents {
        identifier(&item.id, "Inhalts-ID")?;
        provider_identifier(&item.source, &item.project_id, "Projekt-ID")?;
        provider_identifier(&item.source, &item.version_id, "Versions-ID")?;
        filename(&item.filename)?;
        if !matches!(item.source.as_str(), "modrinth" | "curseforge")
            || !matches!(
                item.kind.as_str(),
                "mod" | "shader" | "resourcepack" | "datapack"
            )
        {
            return Err("Die Inhaltsquelle oder der Inhaltstyp wird nicht unterstützt.".into());
        }
        if !matches!(item.status.as_str(), "planned" | "downloaded") {
            return Err("Der lokale Downloadstatus eines Inhalts ist ungültig.".into());
        }
        if item.sha512.as_ref().is_some_and(|hash| !valid_hash(hash)) {
            return Err("Die SHA-512-Prüfsumme eines Inhalts ist ungültig.".into());
        }
        if item.sha1.as_ref().is_some_and(|hash| {
            hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("Die SHA-1-Prüfsumme eines Inhalts ist ungültig.".into());
        }
        if !projects.insert(project_key(&item.source, &item.project_id)) || !ids.insert(&item.id) {
            return Err("Ein Projekt oder eine Inhalts-ID kommt im Profil mehrfach vor.".into());
        }
        if !filenames.insert(item.filename.to_ascii_lowercase()) {
            return Err("Zwei Inhalte im Profil verwenden denselben Dateinamen.".into());
        }
    }
    Ok(())
}

fn client_environment(value: &str) -> bool {
    !matches!(value, "server_only" | "dedicated_server_only")
}

fn artifact_kind(version: &ApiVersion, project: &ApiProject) -> Result<String, String> {
    if version.source == "curseforge" {
        return Ok(project.project_type.clone());
    }
    let contains = |loader: &str| version.loaders.iter().any(|l| l == loader);
    if contains("fabric") || contains("forge") || contains("neoforge") || contains("quilt") {
        return Ok("mod".into());
    }
    if contains("iris") || contains("optifine") || contains("canvas") || contains("vanilla") {
        return Ok("shader".into());
    }
    if contains("minecraft") {
        return Ok("resourcepack".into());
    }
    if contains("datapack") {
        return Ok("datapack".into());
    }
    if matches!(
        project.project_type.as_str(),
        "mod" | "shader" | "resourcepack"
    ) {
        return Ok(project.project_type.clone());
    }
    Err(format!(
        "„{}“ hat keinen unterstützten Client-Inhaltstyp.",
        project.title
    ))
}

fn compatible(
    version: &ApiVersion,
    project: &ApiProject,
    game: &str,
    loader: &str,
    kind: &str,
) -> Result<(), String> {
    if !project.distribution_allowed {
        return Err(format!(
            "Der Autor erlaubt die Verteilung von „{}“ durch SyncMC nicht.",
            project.title
        ));
    }
    if !version.game_versions.iter().any(|v| v == game) {
        return Err(format!(
            "„{}“ {} unterstützt Minecraft {game} nicht.",
            project.title, version.version_number
        ));
    }
    if project.id != version.project_id {
        return Err("Eine Inhaltsversion gehört zum falschen Projekt.".into());
    }
    let inferred = artifact_kind(version, project)?;
    if inferred != kind {
        return Err(format!(
            "Die ausgewählte Datei von „{}“ hat den falschen Inhaltstyp.",
            project.title
        ));
    }
    directory(kind)?;
    let allowed: &[&str] = match kind {
        "mod" => {
            if loader != "fabric" {
                return Err("Moddateien werden zunächst nur für Fabric unterstützt.".into());
            }
            &["fabric"]
        }
        "shader" => &["iris", "canvas", "optifine", "vanilla"],
        "resourcepack" => &["minecraft"],
        _ => &[],
    };
    if (version.source != "curseforge" || kind == "mod")
        && !version
            .loaders
            .iter()
            .any(|l| allowed.contains(&l.as_str()))
    {
        return Err(format!(
            "„{}“ {} passt nicht zu {loader}.",
            project.title, version.version_number
        ));
    }
    if kind == "mod" {
        let supported = version
            .environment
            .as_deref()
            .map(client_environment)
            .unwrap_or_else(|| {
                project.client_side.as_deref() != Some("unsupported")
                    && (project.environment.is_empty()
                        || project.environment.iter().any(|e| client_environment(e)))
            });
        if !supported {
            return Err(format!(
                "„{}“ ist ein reiner Server-Mod und kann nicht im Client installiert werden.",
                project.title
            ));
        }
    }
    Ok(())
}

fn downloadable_file(file: &ApiFile, kind: &str) -> bool {
    let extension = if kind == "mod" { ".jar" } else { ".zip" };
    file.filename.to_ascii_lowercase().ends_with(extension)
        && !matches!(
            file.file_type.as_deref(),
            Some("sources-jar" | "dev-jar" | "javadoc-jar" | "signature")
        )
}

fn cdn_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Ungültige Modrinth-Downloadadresse.")?;
    if url.scheme() != "https"
        || url.host_str() != Some("cdn.modrinth.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || !url.path().starts_with("/data/")
    {
        return Err(
            "Moddateien dürfen ausschließlich vom offiziellen Modrinth-CDN geladen werden.".into(),
        );
    }
    Ok(url)
}

fn selected_file(
    version: &ApiVersion,
    selection: Option<&ContentSelection>,
    kind: &str,
) -> Result<ApiFile, String> {
    let file = if let Some(selection) = selection {
        version
            .files
            .iter()
            .find(|file| file.filename == selection.filename)
            .ok_or("Die vorgemerkte Datei gehört nicht zur ausgewählten Dateiversion.")?
    } else {
        version
            .files
            .iter()
            .filter(|file| downloadable_file(file, kind))
            .find(|file| file.primary)
            .or_else(|| {
                version
                    .files
                    .iter()
                    .find(|file| downloadable_file(file, kind))
            })
            .ok_or("Für eine erforderliche Abhängigkeit gibt es keine installierbare Datei.")?
    };
    filename(&file.filename)?;
    if !downloadable_file(file, kind) {
        return Err("Die ausgewählte Datei hat kein unterstütztes Dateiformat.".into());
    }
    if file.source != version.source {
        return Err("Die Dateiquelle stimmt nicht mit ihrer Version überein.".into());
    }
    if file.size == 0 || file.size > MAX_ARTIFACT_SIZE {
        return Err("Die angekündigte Dateigröße wird nicht unterstützt.".into());
    }
    if file.source == "curseforge" {
        let hash = file
            .hashes
            .get("sha1")
            .filter(|hash| hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("CurseForge hat keine gültige offizielle SHA-1-Prüfsumme geliefert.")?;
        if selection
            .and_then(|item| item.sha1.as_ref())
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(hash))
        {
            return Err("Die vorgemerkte SHA-1-Prüfsumme stimmt nicht mit den offiziellen CurseForge-Metadaten überein.".into());
        }
        return Ok(file.clone());
    }
    cdn_url(&file.url)?;
    let hash = file
        .hashes
        .get("sha512")
        .filter(|h| valid_hash(h))
        .ok_or("Modrinth hat keine gültige SHA-512-Prüfsumme geliefert.")?;
    if selection
        .and_then(|s| s.sha512.as_ref())
        .is_some_and(|s| !s.eq_ignore_ascii_case(hash))
    {
        return Err("Die vorgemerkte Prüfsumme stimmt nicht mehr mit Modrinth überein. Bitte die Version erneut auswählen.".into());
    }
    if file.size == 0 || file.size > MAX_ARTIFACT_SIZE {
        return Err("Die angekündigte Moddateigröße wird nicht unterstützt.".into());
    }
    Ok(file.clone())
}

#[derive(Clone, Debug)]
struct Resolved {
    selection: ContentSelection,
    version: ApiVersion,
    file: ApiFile,
    fixed: bool,
}

enum ResolutionAttempt {
    Complete(Vec<Resolved>),
    Restart { project: String, version: String },
}

#[derive(Clone)]
struct Requirement {
    source: String,
    project: Option<String>,
    version: Option<String>,
    root: bool,
    from: String,
}

fn version_conflict(project: &str, left: &str, right: &str) -> String {
    format!("Versionskonflikt bei „{project}“: {left} und {right} werden gleichzeitig benötigt. Wähle zueinander passende Modversionen.")
}

async fn resolve<R: SourcesMetadata>(
    repo: &mut R,
    game: &str,
    loader: &str,
    contents: &[ContentSelection],
) -> Result<Vec<Resolved>, String> {
    let mut preferences = BTreeMap::new();
    let mut seen = BTreeSet::new();
    seen.insert(preferences.clone());
    for _ in 0..64 {
        match resolve_once(repo, game, loader, contents, &preferences).await? {
            ResolutionAttempt::Complete(resolved) => return Ok(resolved),
            ResolutionAttempt::Restart { project, version } => {
                preferences.insert(project, version);
                if !seen.insert(preferences.clone()) {
                    return Err("Die Pflichtabhängigkeiten verlangen wiederholt wechselnde Versionen. Es wurde keine eindeutige kompatible Auswahl gefunden; die Instanz bleibt unverändert.".into());
                }
            }
        }
    }
    Err("Die Abhängigkeitsauflösung benötigt zu viele Versionswechsel. Bitte wähle zueinander passende Modversionen; die Instanz bleibt unverändert.".into())
}

async fn resolve_once<R: SourcesMetadata>(
    repo: &mut R,
    game: &str,
    loader: &str,
    contents: &[ContentSelection],
    preferences: &BTreeMap<String, String>,
) -> Result<ResolutionAttempt, String> {
    validate_selections(contents)?;
    if let Some(item) = contents
        .iter()
        .find(|item| item.enabled && !item.dependency && item.version_id.starts_with("demo-"))
    {
        return Err(format!("„{}“ ist noch ein Beispiel. Wähle im Modkatalog zuerst eine echte Dateiversion oder entferne den Beispieleintrag.",item.name));
    }
    let pins: BTreeMap<String, &ContentSelection> = contents
        .iter()
        .map(|s| (project_key(&s.source, &s.project_id), s))
        .collect();
    let mut queue: VecDeque<Requirement> = contents
        .iter()
        .filter(|s| s.enabled && !s.dependency)
        .map(|s| Requirement {
            source: s.source.clone(),
            project: Some(s.project_id.clone()),
            version: Some(s.version_id.clone()),
            root: true,
            from: s.name.clone(),
        })
        .collect();
    // Resolve all exact constraints before choosing versions for unbound project dependencies.
    let mut project_queue = VecDeque::new();
    let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
    let mut edges = 0;
    while let Some(requirement) = queue.pop_front().or_else(|| project_queue.pop_front()) {
        edges += 1;
        if edges > MAX_CONTENTS * 8 || resolved.len() > MAX_CONTENTS {
            return Err("Die Abhängigkeitsliste ist zu groß.".into());
        }
        if let Some(project_id) = &requirement.project {
            provider_identifier(&requirement.source, project_id, "Abhängigkeits-Projekt-ID")?;
            let key = project_key(&requirement.source, project_id);
            if let Some(existing) = resolved.get_mut(&key) {
                if requirement
                    .version
                    .as_ref()
                    .is_some_and(|v| v != &existing.version.id)
                {
                    if existing.fixed {
                        return Err(version_conflict(
                            &existing.selection.name,
                            &existing.version.id,
                            requirement.version.as_deref().unwrap(),
                        ));
                    }
                    return Ok(ResolutionAttempt::Restart {
                        project: key,
                        version: requirement.version.clone().unwrap(),
                    });
                }
                existing.fixed |= requirement.version.is_some();
                continue;
            }
        }
        let pinned = requirement
            .project
            .as_deref()
            .and_then(|id| pins.get(&project_key(&requirement.source, id)).copied());
        if let Some(pin) = pinned {
            if !pin.enabled {
                return Err(format!("„{}“ wird von „{}“ benötigt, ist aber deaktiviert. Aktiviere die Abhängigkeit oder entferne den anfordernden Mod.", pin.name, requirement.from));
            }
            if let Some(exact) = &requirement.version {
                if exact != &pin.version_id && !pin.dependency {
                    return Err(version_conflict(&pin.name, &pin.version_id, exact));
                }
            }
        }
        let pinned = pinned.filter(|pin| {
            requirement
                .version
                .as_ref()
                .is_none_or(|version| version == &pin.version_id)
        });
        let version_id = requirement
            .version
            .as_deref()
            .or_else(|| {
                requirement
                    .project
                    .as_ref()
                    .and_then(|project| preferences.get(&project_key(&requirement.source, project)))
                    .map(String::as_str)
            })
            .or_else(|| pinned.map(|pin| pin.version_id.as_str()));
        let (version, project) = if let Some(version_id) = version_id {
            let version = repo
                .version_for(
                    &requirement.source,
                    requirement.project.as_deref(),
                    version_id,
                )
                .await?;
            provider_identifier(&requirement.source, &version.id, "Versions-ID")?;
            provider_identifier(&requirement.source, &version.project_id, "Projekt-ID")?;
            if version.source != requirement.source
                || version.id != version_id
                || requirement
                    .project
                    .as_ref()
                    .is_some_and(|p| p != &version.project_id)
            {
                return Err(format!(
                    "Die Abhängigkeit von „{}“ gehört nicht zum erwarteten Projekt ihrer Quelle.",
                    requirement.from
                ));
            }
            let project = repo
                .project_for(&requirement.source, &version.project_id)
                .await?;
            (version, project)
        } else {
            let project_id = requirement.project.as_deref().ok_or_else(|| format!("Eine Pflichtabhängigkeit von „{}“ verweist auf keine Modrinth-Version. Bitte installiere eine andere Version dieses Mods.", requirement.from))?;
            let project = repo.project_for(&requirement.source, project_id).await?;
            let mut candidates = repo
                .versions_for(&requirement.source, project_id, game, loader)
                .await?;
            candidates.retain(|version| {
                version.source == requirement.source && version.project_id == project_id
            });
            candidates.sort_by(|a, b| {
                let rank = |v: &ApiVersion| match v.version_type.as_str() {
                    "release" => 0,
                    "beta" => 1,
                    _ => 2,
                };
                rank(a)
                    .cmp(&rank(b))
                    .then_with(|| b.date_published.cmp(&a.date_published))
                    .then_with(|| a.id.cmp(&b.id))
            });
            let version = candidates.into_iter().find(|version| {
                artifact_kind(version, &project).is_ok_and(|kind| compatible(version, &project, game, loader, &kind).is_ok())
            }).ok_or_else(|| format!("Für die Pflichtabhängigkeit „{}“ gibt es keine passende Client-Version für Minecraft {game} / {loader}.", project.title))?;
            (version, project)
        };
        // Version-only dependencies may reveal a project that is already explicitly pinned.
        let pinned = pins
            .get(&project_key(&version.source, &version.project_id))
            .copied();
        if let Some(pin) = pinned {
            if !pin.enabled {
                return Err(format!(
                    "Die erforderliche Abhängigkeit „{}“ ist deaktiviert.",
                    pin.name
                ));
            }
            if pin.version_id != version.id && !pin.dependency {
                return Err(version_conflict(&pin.name, &pin.version_id, &version.id));
            }
        }
        // A changed root can require a new exact dependency version. Prior automatic pins
        // are preferences; explicit user selections remain hard constraints.
        let pinned = pinned.filter(|pin| pin.version_id == version.id);
        let key = project_key(&version.source, &version.project_id);
        if let Some(existing) = resolved.get_mut(&key) {
            if existing.version.id != version.id {
                if existing.fixed {
                    return Err(version_conflict(
                        &project.title,
                        &existing.version.id,
                        &version.id,
                    ));
                }
                return Ok(ResolutionAttempt::Restart {
                    project: key,
                    version: version.id.clone(),
                });
            }
            existing.fixed |= requirement.version.is_some();
            continue;
        }
        let kind = pinned
            .map(|s| s.kind.clone())
            .unwrap_or(artifact_kind(&version, &project)?);
        compatible(&version, &project, game, loader, &kind)?;
        let file = selected_file(&version, pinned, &kind)?;
        let selection = ContentSelection {
            id: pinned.map(|s| s.id.clone()).unwrap_or_else(|| {
                if version.source == "modrinth" {
                    format!("dep-{}", version.project_id)
                } else {
                    format!("dep-{}-{}", version.source, version.project_id)
                }
            }),
            project_id: version.project_id.clone(),
            version_id: version.id.clone(),
            name: project.title.chars().take(160).collect(),
            version: version.version_number.chars().take(160).collect(),
            filename: file.filename.clone(),
            icon_url: project
                .icon_url
                .clone()
                .or_else(|| pinned.and_then(|selection| selection.icon_url.clone()))
                .filter(|url| url.starts_with("https://")),
            kind,
            enabled: true,
            sha512: file
                .hashes
                .get("sha512")
                .map(|h| h.to_ascii_lowercase())
                .or_else(|| pinned.and_then(|selection| selection.sha512.clone())),
            sha1: file
                .hashes
                .get("sha1")
                .map(|hash| hash.to_ascii_lowercase()),
            source: version.source.clone(),
            status: "downloaded".into(),
            dependency: !requirement.root && pinned.is_none_or(|s| s.dependency),
        };
        for dependency in &version.dependencies {
            if dependency.dependency_type != "required" {
                continue;
            }
            if dependency.project_id.is_none() && dependency.version_id.is_none() {
                return Err(format!("„{}“ benötigt die externe Datei „{}“. Diese Abhängigkeit kann nicht automatisch von Modrinth installiert werden.", selection.name, dependency.file_name.as_deref().unwrap_or("unbekannt")));
            }
            let required = Requirement {
                source: version.source.clone(),
                project: dependency.project_id.clone(),
                version: dependency.version_id.clone(),
                root: false,
                from: selection.name.clone(),
            };
            if required.version.is_some() {
                queue.push_back(required);
            } else {
                project_queue.push_back(required);
            }
        }
        resolved.insert(
            project_key(&version.source, &version.project_id),
            Resolved {
                selection,
                version,
                file,
                fixed: requirement.version.is_some() || pinned.is_some_and(|pin| !pin.dependency),
            },
        );
    }
    for item in resolved.values() {
        for dependency in item
            .version
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == "incompatible")
        {
            let conflict = if let Some(project) = &dependency.project_id {
                resolved
                    .get(&project_key(&item.version.source, project))
                    .filter(|other| {
                        dependency
                            .version_id
                            .as_ref()
                            .is_none_or(|v| v == &other.version.id)
                    })
            } else {
                resolved.values().find(|other| {
                    other.version.source == item.version.source
                        && dependency
                            .version_id
                            .as_ref()
                            .is_some_and(|v| v == &other.version.id)
                })
            };
            if let Some(other) = conflict {
                return Err(format!("„{}“ ist mit „{}“ {} nicht kompatibel. Entferne einen der beiden Inhalte oder wähle andere Versionen.", item.selection.name, other.selection.name, other.selection.version));
            }
        }
    }
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for item in resolved.values() {
        if !paths.insert(content_path(&item.selection)?.to_ascii_lowercase())
            || !ids.insert(item.selection.id.clone())
        {
            return Err(
                "Zwei Inhalte erzeugen denselben Dateinamen oder dieselbe Inhalts-ID.".into(),
            );
        }
    }
    Ok(ResolutionAttempt::Complete(
        resolved.into_values().collect(),
    ))
}

fn content_path(selection: &ContentSelection) -> Result<String, String> {
    filename(&selection.filename)?;
    Ok(format!(
        "{}/{}",
        directory(&selection.kind)?,
        selection.filename
    ))
}

fn checked_relative(value: &Path) -> Result<(), String> {
    if value.as_os_str().is_empty() {
        return Err("Leerer Pfad in der Inhaltsinstallation.".into());
    }
    for component in value.components() {
        match component {
            Component::Normal(name) => filename(&name.to_string_lossy())?,
            _ => return Err("Unsicherer Pfad in der Inhaltsinstallation.".into()),
        }
    }
    Ok(())
}

fn managed_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    checked_relative(relative)?;
    let mut path = root.to_path_buf();
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Der Minecraft-Datenordner darf kein symbolischer Link sein.".into());
    }
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Ein Pfad der Inhaltsinstallation enthält einen symbolischen Link.".into());
        }
    }
    Ok(path)
}

fn prepare_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let path = managed_path(root, relative)?;
    fs::create_dir_all(path.parent().ok_or("Ungültiger Installationspfad.")?)
        .map_err(|e| format!("Inhaltsordner konnte nicht erstellt werden: {e}"))?;
    Ok(path)
}

fn replace_file(temp: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if destination.exists() {
        fs::remove_file(destination)
            .map_err(|e| format!("Inhaltsdatei konnte nicht ersetzt werden: {e}"))?;
    }
    fs::rename(temp, destination)
        .map_err(|e| format!("Inhaltsdatei konnte nicht gespeichert werden: {e}"))
}

fn hash_file(path: &Path, hash: &str, size: u64) -> Result<bool, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Inhaltsdatei konnte nicht gelesen werden: {e}")),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() != size {
        return Ok(false);
    }
    let mut digest = Sha512::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("Inhaltsprüfsumme konnte nicht geprüft werden: {e}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()).eq_ignore_ascii_case(hash))
}

async fn cache_file(root: &Path, file: &ApiFile) -> Result<PathBuf, String> {
    let hash = file
        .hashes
        .get("sha512")
        .ok_or("SHA-512-Prüfsumme fehlt.")?
        .to_ascii_lowercase();
    if !valid_hash(&hash) {
        return Err("Ungültige SHA-512-Prüfsumme.".into());
    }
    let relative = PathBuf::from("content-cache").join(&hash);
    let cached = prepare_file(root, &relative)?;
    if hash_file(&cached, &hash, file.size)? {
        return Ok(cached);
    }
    let temp = prepare_file(root, &relative.with_extension("syncmc-part"))?;
    let mut last_error = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(300 * attempt)).await;
        }
        let result = async {
            let mut response = client()?
                .get(cdn_url(&file.url)?)
                .send()
                .await
                .map_err(|_| "Der Modrinth-Download ist fehlgeschlagen.")?;
            if !response.status().is_success() {
                return Err(format!(
                    "Modrinth-Download fehlgeschlagen (HTTP {}).",
                    response.status().as_u16()
                ));
            }
            let mut output = File::create(&temp)
                .map_err(|e| format!("Die Moddatei konnte nicht geschrieben werden: {e}"))?;
            let mut digest = Sha512::new();
            let mut written = 0_u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Der Modrinth-Download wurde unterbrochen.")?
            {
                written += chunk.len() as u64;
                if written > file.size || written > MAX_ARTIFACT_SIZE {
                    return Err("Die Moddatei ist größer als angekündigt.".into());
                }
                digest.update(&chunk);
                output
                    .write_all(&chunk)
                    .map_err(|e| format!("Die Moddatei konnte nicht gespeichert werden: {e}"))?;
            }
            if written != file.size || format!("{:x}", digest.finalize()) != hash {
                return Err(
                    "SHA-512-Prüfung fehlgeschlagen. Die Moddatei wird erneut geladen.".into(),
                );
            }
            output
                .sync_all()
                .map_err(|e| format!("Die Moddatei konnte nicht gesichert werden: {e}"))?;
            drop(output);
            replace_file(&temp, &cached)?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => return Ok(cached),
            Err(error) => {
                last_error = error;
                let _ = fs::remove_file(&temp);
            }
        }
    }
    Err(format!("{}: {last_error}", file.filename))
}

/// CurseForge artifacts are only kept in the installed instance. This operation's working
/// files are removed on both success and error and are never reused as a download cache.
struct ContentWork {
    directory: Option<PathBuf>,
}
impl ContentWork {
    fn new(root: &Path, needed: bool) -> Result<Self, String> {
        if !needed {
            return Ok(Self { directory: None });
        }
        let relative = PathBuf::from("content-work").join(uuid::Uuid::new_v4().to_string());
        let directory = managed_path(root, &relative)?;
        fs::create_dir_all(&directory).map_err(|error| {
            format!("Der temporäre Inhaltsordner konnte nicht erstellt werden: {error}")
        })?;
        Ok(Self {
            directory: Some(directory),
        })
    }
    fn file(&self, index: usize) -> Result<PathBuf, String> {
        self.directory
            .as_ref()
            .map(|directory| directory.join(format!("curseforge-{index}.download")))
            .ok_or_else(|| "Der temporäre CurseForge-Arbeitsordner fehlt.".into())
    }
}
impl Drop for ContentWork {
    fn drop(&mut self) {
        if let Some(directory) = &self.directory {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

/// A locally computed SHA-512 proves installed bytes; the fresh provider SHA-1 proves which
/// official CurseForge file was selected. An imported extra SHA-512 is also checked if present.
fn verified_curseforge_bytes(
    path: &Path,
    file: &ApiFile,
    extra_sha512: Option<&str>,
) -> Result<Option<String>, String> {
    let expected = file
        .hashes
        .get("sha1")
        .filter(|hash| hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("Die offizielle CurseForge-SHA-1-Prüfsumme fehlt.")?;
    let mut input = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Die CurseForge-Datei konnte nicht geprüft werden: {error}"
            ))
        }
    };
    let metadata = input.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() != file.size {
        return Ok(None);
    }
    let mut sha1 = Sha1::new();
    let mut sha512 = Sha512::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        sha1.update(&buffer[..count]);
        sha512.update(&buffer[..count]);
    }
    if !format!("{:x}", sha1.finalize()).eq_ignore_ascii_case(expected) {
        return Ok(None);
    }
    let sha512 = format!("{:x}", sha512.finalize());
    if extra_sha512.is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha512)) {
        return Err("Die zusätzliche SHA-512-Prüfsumme im Profil passt nicht zu den offiziell geprüften CurseForge-Dateibytes. Die Instanz wurde nicht verändert.".into());
    }
    Ok(Some(sha512))
}

async fn prepare_curseforge_file(
    work: &ContentWork,
    installed: &Path,
    item: &mut Resolved,
    index: usize,
) -> Result<PathBuf, String> {
    let extra = item.selection.sha512.clone();
    if let Some(hash) = verified_curseforge_bytes(installed, &item.file, extra.as_deref())? {
        item.selection.sha512 = Some(hash);
        return Ok(installed.to_path_buf());
    }
    let target = work.file(index)?;
    let raw = item.file.curseforge_file.as_ref().ok_or(
        "Die ursprünglichen CurseForge-Metadaten fehlen. Bitte die Version erneut auswählen.",
    )?;
    crate::curseforge::download_file(raw, &target).await?;
    let hash=verified_curseforge_bytes(&target,&item.file,extra.as_deref())?.ok_or("Die CurseForge-Datei stimmt nicht mit ihrer offiziellen SHA-1-Prüfsumme überein. Die Instanz wurde nicht verändert.")?;
    item.selection.sha512 = Some(hash);
    Ok(target)
}

fn distinct_mod_binaries(resolved: &[Resolved]) -> Result<(), String> {
    let mut binaries = BTreeMap::new();
    for item in resolved.iter().filter(|item| item.selection.kind == "mod") {
        let hash = item
            .selection
            .sha512
            .as_deref()
            .ok_or("Für einen Mod fehlt die lokale Inhaltsprüfsumme.")?;
        let key = project_key(&item.selection.source, &item.selection.project_id);
        if let Some((previous_key, previous_name)) = binaries.insert(
            hash.to_ascii_lowercase(),
            (key.clone(), item.selection.name.clone()),
        ) {
            if previous_key != key {
                return Err(format!("„{}“ und „{previous_name}“ enthalten dieselben Moddateibytes unter verschiedenen Projekten oder Quellen. Entferne einen Eintrag; SyncMC lädt sie nicht doppelt und führt keine automatische Quellzuordnung durch.",item.selection.name));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LockedFile {
    path: String,
    sha512: String,
    size: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentLock {
    format: String,
    version: u8,
    instance_id: String,
    game_version: String,
    loader: String,
    desired_sha512: String,
    contents: Vec<ContentSelection>,
    files: Vec<LockedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<Value>,
}

/// Ownership survives an interrupted multi-file commit; readiness still comes only from the lock.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingContent {
    format: String,
    version: u8,
    instance_id: String,
    files: Vec<LockedFile>,
}

fn fingerprint(contents: &[ContentSelection]) -> Result<String, String> {
    // Keep the original tuple byte-for-byte for existing Modrinth locks and backups.
    // CurseForge's computed local SHA-512 is installation proof, not a provider version pin.
    if contents
        .iter()
        .any(|selection| selection.source != "modrinth")
    {
        let mut rows = contents
            .iter()
            .map(|selection| {
                (
                    &selection.source,
                    &selection.project_id,
                    &selection.version_id,
                    &selection.filename,
                    &selection.kind,
                    selection.enabled,
                    if selection.source == "curseforge" {
                        selection
                            .sha1
                            .as_ref()
                            .map(|hash| hash.to_ascii_lowercase())
                    } else {
                        selection
                            .sha512
                            .as_ref()
                            .map(|hash| hash.to_ascii_lowercase())
                    },
                    selection.dependency,
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
        return Ok(format!(
            "{:x}",
            Sha512::digest(
                serde_json::to_vec(&("syncmc-provider-content-v2", rows))
                    .map_err(|error| error.to_string())?
            )
        ));
    }
    let mut rows = contents
        .iter()
        .map(|s| {
            (
                &s.project_id,
                &s.version_id,
                &s.filename,
                &s.kind,
                s.enabled,
                s.sha512.as_ref().map(|h| h.to_ascii_lowercase()),
                s.dependency,
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    Ok(format!(
        "{:x}",
        Sha512::digest(serde_json::to_vec(&rows).map_err(|e| e.to_string())?)
    ))
}

fn instance_relative(instance_id: &str) -> Result<PathBuf, String> {
    identifier(instance_id, "Instanz-ID")?;
    Ok(PathBuf::from("instances").join(instance_id))
}

fn lock_relative(instance_id: &str) -> Result<PathBuf, String> {
    Ok(instance_relative(instance_id)?.join("content-lock.json"))
}

fn pending_relative(instance_id: &str) -> Result<PathBuf, String> {
    Ok(instance_relative(instance_id)?.join("content-pending.json"))
}

fn validate_locked_file(file: &LockedFile) -> Result<(), String> {
    let parts: Vec<&str> = file.path.split('/').collect();
    if parts.len() != 2 || !matches!(parts[0], "mods" | "shaderpacks" | "resourcepacks") {
        return Err("Der Inhaltsstatus enthält einen ungültigen Dateipfad.".into());
    }
    filename(parts[1])?;
    if !valid_hash(&file.sha512) || file.size == 0 || file.size > MAX_ARTIFACT_SIZE {
        return Err("Der Inhaltsstatus enthält eine ungültige Datei.".into());
    }
    Ok(())
}

fn read_pending(root: &Path, instance_id: &str) -> Result<Vec<LockedFile>, String> {
    let path = managed_path(root, &pending_relative(instance_id)?)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "Die unterbrochene Inhaltsinstallation konnte nicht gelesen werden: {e}"
            ))
        }
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_METADATA_SIZE as u64 {
        return Err("Der ausstehende Inhaltsstatus ist zu groß.".into());
    }
    let pending: PendingContent = serde_json::from_reader(file).map_err(|_| "Der ausstehende Inhaltsstatus ist beschädigt. Sichere den Instanzordner und prüfe content-pending.json.")?;
    if pending.format != "syncmc-content-pending"
        || pending.version != 1
        || pending.instance_id != instance_id
        || pending.files.len() > MAX_CONTENTS * 8
    {
        return Err("Der ausstehende Inhaltsstatus hat ein ungültiges Format.".into());
    }
    for file in &pending.files {
        validate_locked_file(file)?;
    }
    Ok(pending.files)
}

fn read_lock(root: &Path, instance_id: &str) -> Result<Option<ContentLock>, String> {
    let path = managed_path(root, &lock_relative(instance_id)?)?;
    read_lock_file(&path, instance_id)
}

fn read_lock_file(path: &Path, instance_id: &str) -> Result<Option<ContentLock>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Der lokale Inhaltsstatus konnte nicht gelesen werden: {e}"
            ))
        }
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("Der lokale Inhaltsstatus ist zu groß.".into());
    }
    let lock: ContentLock = serde_json::from_reader(file).map_err(|_| "Der lokale Inhaltsstatus ist beschädigt. Sichere den Instanzordner und entferne content-lock.json, bevor du erneut installierst.")?;
    if lock.format != "syncmc-content-lock"
        || lock.version != 1
        || lock.instance_id != instance_id
        || lock.files.len() > MAX_CONTENTS
    {
        return Err("Der lokale Inhaltsstatus hat ein ungültiges Format.".into());
    }
    validate_selections(&lock.contents)?;
    let expected: BTreeMap<String, String> = lock
        .contents
        .iter()
        .filter(|s| s.enabled)
        .map(|s| {
            Ok((
                content_path(s)?,
                s.sha512
                    .clone()
                    .filter(|h| valid_hash(h))
                    .ok_or("Im lokalen Inhaltsstatus fehlt eine Prüfsumme.")?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let mut paths = BTreeSet::new();
    if expected.len() != lock.files.len() {
        return Err("Der lokale Inhaltsstatus ist unvollständig.".into());
    }
    for file in &lock.files {
        if !valid_hash(&file.sha512)
            || file.size == 0
            || file.size > MAX_ARTIFACT_SIZE
            || expected
                .get(&file.path)
                .is_none_or(|hash| !hash.eq_ignore_ascii_case(&file.sha512))
            || !paths.insert(file.path.to_ascii_lowercase())
        {
            return Err("Der lokale Inhaltsstatus enthält ungültige Dateien.".into());
        }
    }
    if lock.desired_sha512 != fingerprint(&lock.contents)? {
        return Err("Der lokale Inhaltsstatus enthält eine ungültige Profilprüfsumme.".into());
    }
    if let Some(profile) = &lock.profile {
        let profile = crate::backup::normalize_profile(profile.clone(), instance_id)?;
        let selections: Vec<ContentSelection> = serde_json::from_value(profile["contents"].clone())
            .map_err(|_| "Ungültige Inhalte im lokalen Profil.")?;
        if profile["mcVersion"] != lock.game_version
            || profile["loader"] != lock.loader
            || fingerprint(&selections)? != lock.desired_sha512
        {
            return Err("Das lokal gespeicherte Profil passt nicht zum Inhaltsstatus.".into());
        }
    }
    Ok(Some(lock))
}

/// Prefer the last installed profile to a pending update candidate, so rollback always restores
/// the old exact pins. Legacy locks keep their old contents even when only a new UI profile exists.
pub(crate) fn profile_for_snapshot(
    root: &Path,
    instance_id: &str,
    fallback: Option<Value>,
) -> Result<Value, String> {
    let fallback = fallback
        .map(|profile| crate::backup::normalize_profile(profile, instance_id))
        .transpose()?;
    let lock = read_lock(root, instance_id)?;
    let mut profile = if let Some(lock) = &lock {
        lock.profile
            .clone()
            .or_else(|| fallback.clone())
            .unwrap_or_else(|| {
                crate::backup::default_profile(
                    instance_id,
                    &lock.game_version,
                    &lock.loader,
                    &lock.contents,
                )
            })
    } else {
        fallback
            .clone()
            .ok_or("Für diese Instanz fehlt ein Profil für die Sicherung.")?
    };
    // These fields can change independently of installed content. Keep the user's current
    // name, cover and server favourites while the lock below supplies the old runtime pins.
    if let Some(current) = &fallback {
        for field in ["name", "cover", "servers"] {
            profile[field] = current[field].clone();
        }
    }
    if let Some(lock) = lock {
        profile["id"] = serde_json::json!(instance_id);
        profile["mcVersion"] = serde_json::json!(lock.game_version);
        profile["loader"] = serde_json::json!(lock.loader);
        profile["contents"] = serde_json::to_value(&lock.contents).map_err(|e| e.to_string())?;
        // A legacy fallback's loader version might already be the update candidate.
        if lock.profile.is_none() {
            let installed = managed_path(
                root,
                &instance_relative(instance_id)?.join("installation.json"),
            )?;
            let report = File::open(installed)
                .ok()
                .and_then(|file| serde_json::from_reader::<_, Value>(file).ok());
            if let Some(report) = report.filter(|report| {
                report["versionId"] == profile["mcVersion"]
                    && report["loader"].as_str().unwrap_or("vanilla")
                        == profile["loader"].as_str().unwrap_or("")
            }) {
                if let Some(version) = report
                    .get("loaderVersion")
                    .filter(|version| version.is_string())
                {
                    profile["loaderVersion"] = version.clone();
                } else if let Some(object) = profile.as_object_mut() {
                    object.remove("loaderVersion");
                }
            } else if let Some(object) = profile.as_object_mut() {
                object.remove("loaderVersion");
            }
        }
    }
    crate::backup::normalize_profile(profile, instance_id)
}

fn write_lock(root: &Path, instance_id: &str, lock: &ContentLock) -> Result<(), String> {
    write_json(root, &lock_relative(instance_id)?, lock)
}

fn write_json(root: &Path, relative: &Path, value: &impl Serialize) -> Result<(), String> {
    let target = prepare_file(root, &relative)?;
    let temp = prepare_file(root, &relative.with_extension("syncmc-part"))?;
    let mut file = File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    replace_file(&temp, &target)
}

fn ensure_no_unmanaged_mods(
    root: &Path,
    instance_id: &str,
    files: &[LockedFile],
) -> Result<(), String> {
    let directory = managed_path(root, &instance_relative(instance_id)?)?;
    ensure_no_unmanaged_mods_at(&directory, files)
}

fn ensure_no_unmanaged_mods_at(directory: &Path, files: &[LockedFile]) -> Result<(), String> {
    let path = managed_path(directory, Path::new("mods"))?;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Der Modordner konnte nicht gelesen werden: {e}")),
    };
    let known: BTreeSet<String> = files.iter().map(|f| f.path.to_ascii_lowercase()).collect();
    let mut seen = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase().ends_with(".jar") {
            let key = format!("mods/{name}").to_ascii_lowercase();
            if !known.contains(&key) || !seen.insert(key) {
                return Err(format!("Im Modordner liegt die nicht im Profil erfasste Datei „{name}“. Verschiebe sie vor dem Start aus dem Modordner; SyncMC löscht fremde Dateien nicht."));
            }
        }
    }
    Ok(())
}

/// Recheck a snapshot's semantic lock and exact bytes before an instance directory swap.
pub(crate) fn verify_snapshot_contents(directory: &Path, profile: &Value) -> Result<(), String> {
    let instance = profile["id"]
        .as_str()
        .ok_or("Im Sicherungsprofil fehlt die Instanz-ID.")?;
    let contents: Vec<ContentSelection> = serde_json::from_value(profile["contents"].clone())
        .map_err(|_| "Ungültige Inhalte im Sicherungsprofil.")?;
    validate_selections(&contents)?;
    if managed_path(directory, Path::new("content-pending.json"))?.exists() {
        return Err("Die Sicherung enthält eine unterbrochene Inhaltsinstallation.".into());
    }
    let Some(lock) = read_lock_file(
        &managed_path(directory, Path::new("content-lock.json"))?,
        instance,
    )?
    else {
        if contents.iter().any(|item| item.enabled) {
            return Err("Die Sicherung enthält noch nicht installierte Inhalte.".into());
        }
        return ensure_no_unmanaged_mods_at(directory, &[]);
    };
    if lock.game_version != profile["mcVersion"].as_str().unwrap_or("")
        || lock.loader != profile["loader"].as_str().unwrap_or("")
        || lock.desired_sha512 != fingerprint(&contents)?
    {
        return Err("Das Sicherungsprofil passt nicht zum gesicherten Inhaltsstatus.".into());
    }
    for file in &lock.files {
        let target = managed_path(directory, Path::new(&file.path))?;
        if !hash_file(&target, &file.sha512, file.size)? {
            return Err(format!(
                "Die gesicherte Moddatei „{}“ passt nicht zu ihrem Inhaltsstatus.",
                file.path
            ));
        }
    }
    ensure_no_unmanaged_mods_at(directory, &lock.files)
}

/// Checks both the desired profile and every installed artifact immediately before process launch.
/// The caller holds CONTENT_LOCK while checking and spawning to avoid racing an installation.
pub fn verify_installation(
    root: &Path,
    instance_id: &str,
    game_version: &str,
    loader: &str,
    contents: &[ContentSelection],
) -> Result<(), String> {
    crate::backup::ensure_no_pending_restore(root, instance_id)?;
    validate_selections(contents)?;
    if managed_path(root, &pending_relative(instance_id)?)?.exists() {
        return Err("Eine Inhaltsinstallation wurde unterbrochen. Bitte Inhalte erneut installieren und prüfen.".into());
    }
    let Some(lock) = read_lock(root, instance_id)? else {
        if contents.iter().any(|s| s.enabled) {
            return Err("Die Inhalte dieser Instanz wurden noch nicht installiert. Bitte zuerst installieren und prüfen.".into());
        }
        return ensure_no_unmanaged_mods(root, instance_id, &[]);
    };
    if lock.game_version != game_version
        || lock.loader != loader
        || lock.desired_sha512 != fingerprint(contents)?
    {
        return Err("Das Instanzprofil wurde seit der letzten Inhaltsinstallation verändert. Bitte Inhalte erneut installieren und prüfen.".into());
    }
    let base = instance_relative(instance_id)?;
    for file in &lock.files {
        let path = managed_path(root, &base.join(&file.path))?;
        if !hash_file(&path, &file.sha512, file.size)? {
            return Err(format!(
                "„{}“ fehlt oder ist beschädigt. Bitte die Inhalte erneut installieren und prüfen.",
                file.path
            ));
        }
    }
    ensure_no_unmanaged_mods(root, instance_id, &lock.files)
}

#[cfg(test)]
pub async fn install_at(
    root: &Path,
    instance_id: &str,
    game_version: &str,
    loader: &str,
    contents: Vec<ContentSelection>,
) -> Result<ContentReport, String> {
    install_with_progress(
        root,
        instance_id,
        game_version,
        loader,
        contents,
        None,
        Arc::new(|_| {}),
    )
    .await
}

async fn install_with_progress(
    root: &Path,
    instance_id: &str,
    game_version: &str,
    loader: &str,
    contents: Vec<ContentSelection>,
    profile: Option<Value>,
    progress: Arc<dyn Fn(ContentProgress) + Send + Sync>,
) -> Result<ContentReport, String> {
    let _guard = CONTENT_LOCK
        .try_lock()
        .map_err(|_| "Eine Inhaltsinstallation läuft bereits. Bitte warten.")?;
    let base = instance_relative(instance_id)?;
    crate::backup::ensure_no_pending_restore(root, instance_id)?;
    if game_version.is_empty()
        || game_version.len() > 100
        || game_version.chars().any(char::is_control)
    {
        return Err("Ungültige Minecraft-Version.".into());
    }
    if !matches!(loader, "fabric" | "vanilla") {
        return Err("Die erste Modinstallation unterstützt Fabric und Vanilla-Inhalte.".into());
    }
    let mut repo = ProvidersMetadata::default();
    let mut resolved = resolve(&mut repo, game_version, loader, &contents).await?;
    let work = ContentWork::new(
        root,
        resolved
            .iter()
            .any(|item| item.selection.source == "curseforge"),
    )?;
    let mut cached = Vec::new();
    progress(ContentProgress {
        instance_id: instance_id.into(),
        completed_files: 0,
        total_files: resolved.len(),
    });
    let total_files = resolved.len();
    for (index, item) in resolved.iter_mut().enumerate() {
        let prepared = if item.selection.source == "curseforge" {
            let installed = managed_path(root, &base.join(content_path(&item.selection)?))?;
            prepare_curseforge_file(&work, &installed, item, index).await?
        } else {
            cache_file(root, &item.file).await?
        };
        cached.push(prepared);
        progress(ContentProgress {
            instance_id: instance_id.into(),
            completed_files: index + 1,
            total_files,
        });
    }
    distinct_mod_binaries(&resolved)?;
    let mut canonical: Vec<ContentSelection> = contents
        .iter()
        .filter(|s| !s.enabled && !s.dependency)
        .cloned()
        .map(|mut selection| {
            selection.status = "planned".into();
            selection
        })
        .collect();
    canonical.extend(resolved.iter().map(|r| r.selection.clone()));
    canonical.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.project_id.cmp(&b.project_id))
    });
    validate_selections(&canonical)?;
    let mut canonical_profile = crate::backup::normalize_profile(
        profile.unwrap_or_else(|| {
            crate::backup::default_profile(instance_id, game_version, loader, &contents)
        }),
        instance_id,
    )?;
    let requested_profile_contents: Vec<ContentSelection> =
        serde_json::from_value(canonical_profile["contents"].clone())
            .map_err(|_| "Ungültige Inhalte im Instanzprofil.")?;
    if canonical_profile["mcVersion"] != game_version
        || canonical_profile["loader"] != loader
        || fingerprint(&requested_profile_contents)? != fingerprint(&contents)?
    {
        return Err("Das Instanzprofil passt nicht zur angeforderten Inhaltsinstallation.".into());
    }
    canonical_profile["contents"] = serde_json::to_value(&canonical).map_err(|e| e.to_string())?;
    let old = read_lock(root, instance_id)?;
    let mut previous_files = read_pending(root, instance_id)?;
    previous_files.extend(old.iter().flat_map(|lock| lock.files.clone()));
    let mut previous: BTreeMap<String, Vec<LockedFile>> = BTreeMap::new();
    for file in previous_files {
        previous.entry(file.path.clone()).or_default().push(file);
    }
    let desired_files: Vec<LockedFile> = resolved
        .iter()
        .map(|r| {
            Ok(LockedFile {
                path: content_path(&r.selection)?,
                sha512: r
                    .selection
                    .sha512
                    .clone()
                    .ok_or("Eine Inhaltsprüfsumme fehlt.")?,
                size: r.file.size,
            })
        })
        .collect::<Result<_, String>>()?;
    let desired_paths: BTreeSet<&str> = desired_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let old_paths: BTreeSet<&str> = previous.keys().map(String::as_str).collect();
    for previous_path in previous.keys() {
        if managed_path(root, &base.join(previous_path))?.exists()
            && desired_files.iter().any(|file| {
                file.path.eq_ignore_ascii_case(previous_path) && file.path != *previous_path
            })
        {
            return Err(format!("Der Inhaltsdateiname „{previous_path}“ ändert nur die Groß-/Kleinschreibung. Verschiebe die alte Datei vor der Installation, damit sie auf allen Betriebssystemen eindeutig bleibt."));
        }
    }
    // Refuse collisions before downloading or changing any instance file.
    for file in &desired_files {
        let path = managed_path(root, &base.join(&file.path))?;
        if path.exists()
            && !old_paths.contains(file.path.as_str())
            && !hash_file(&path, &file.sha512, file.size)?
        {
            return Err(format!("„{}“ existiert bereits als fremde Datei. Sichere oder verschiebe sie; SyncMC überschreibt sie nicht.", file.path));
        }
    }
    for (previous_path, alternatives) in previous
        .iter()
        .filter(|(path, _)| !desired_paths.contains(path.as_str()))
    {
        let path = managed_path(root, &base.join(previous_path))?;
        let mut owned = !path.exists();
        for file in alternatives {
            owned |= hash_file(&path, &file.sha512, file.size)?;
        }
        if !owned {
            return Err(format!("Die veraltete Inhaltsdatei „{previous_path}“ wurde außerhalb von SyncMC verändert. Sichere oder verschiebe sie vor der Installation."));
        }
    }
    let mut changes = old
        .as_ref()
        .map(|lock| {
            lock.desired_sha512 != fingerprint(&canonical).unwrap_or_default()
                || lock.game_version != game_version
                || lock.loader != loader
        })
        .unwrap_or(!desired_files.is_empty() || !previous.is_empty());
    changes |= managed_path(root, &pending_relative(instance_id)?)?.exists();
    for file in &desired_files {
        if !hash_file(
            &managed_path(root, &base.join(&file.path))?,
            &file.sha512,
            file.size,
        )? {
            changes = true;
        }
    }
    changes |= previous
        .keys()
        .any(|file| !desired_paths.contains(file.as_str()));
    let backup = if changes {
        let mut fallback = canonical_profile.clone();
        if old.is_none() {
            fallback["contents"] = serde_json::json!([]);
        }
        let previous_profile = profile_for_snapshot(root, instance_id, Some(fallback))?;
        let owned_root = root.to_path_buf();
        let owned_instance = instance_id.to_owned();
        Some(tauri::async_runtime::spawn_blocking(move||crate::backup::create_at(&owned_root,&owned_instance,previous_profile,"before-content-change")).await.map_err(|_|"Die Sicherung vor der Inhaltsänderung wurde unterbrochen. Die Moddateien wurden nicht verändert.")??)
    } else {
        None
    };
    // The journal blocks launch and records every file we may replace or remove. On a retry,
    // both old and partially committed artifacts can be recognized without deleting user files.
    let mut pending_files: BTreeMap<(String, String, u64), LockedFile> = BTreeMap::new();
    for file in previous.values().flatten().chain(desired_files.iter()) {
        pending_files.insert(
            (file.path.clone(), file.sha512.clone(), file.size),
            file.clone(),
        );
    }
    if pending_files.len() > MAX_CONTENTS * 8 {
        return Err("Zu viele ausstehende Inhaltsdateien. Bitte den Instanzordner vor weiteren Änderungen prüfen.".into());
    }
    let pending = PendingContent {
        format: "syncmc-content-pending".into(),
        version: 1,
        instance_id: instance_id.into(),
        files: pending_files.into_values().collect(),
    };
    write_json(root, &pending_relative(instance_id)?, &pending)?;
    for (file, cache) in desired_files.iter().zip(cached) {
        let relative = base.join(&file.path);
        let path = prepare_file(root, &relative)?;
        if hash_file(&path, &file.sha512, file.size)? {
            continue;
        }
        let temp = prepare_file(root, &relative.with_extension("syncmc-part"))?;
        fs::copy(cache, &temp)
            .map_err(|e| format!("Moddatei konnte nicht in die Instanz kopiert werden: {e}"))?;
        if !hash_file(&temp, &file.sha512, file.size)? {
            return Err("Die kopierte Moddatei hat eine ungültige Prüfsumme.".into());
        }
        replace_file(&temp, &path)?;
    }
    for previous_path in previous
        .keys()
        .filter(|path| !desired_paths.contains(path.as_str()))
    {
        let path = managed_path(root, &base.join(previous_path))?;
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "Veraltete Moddatei konnte nicht entfernt werden: {e}"
                ))
            }
        }
    }
    ensure_no_unmanaged_mods(root, instance_id, &desired_files)?;
    let lock = ContentLock {
        format: "syncmc-content-lock".into(),
        version: 1,
        instance_id: instance_id.into(),
        game_version: game_version.into(),
        loader: loader.into(),
        desired_sha512: fingerprint(&canonical)?,
        contents: canonical.clone(),
        files: desired_files,
        profile: Some(canonical_profile),
    };
    write_lock(root, instance_id, &lock)?;
    fs::remove_file(managed_path(root, &pending_relative(instance_id)?)?)
        .map_err(|e| format!("Die Inhaltsinstallation konnte nicht abgeschlossen werden: {e}"))?;
    Ok(ContentReport {
        dependency_count: canonical
            .iter()
            .filter(|s| s.enabled && s.dependency)
            .count(),
        files_verified: resolved.len(),
        contents: canonical,
        backup,
    })
}

#[tauri::command]
pub async fn install_instance_content(
    app: tauri::AppHandle,
    instance_id: String,
    game_version: String,
    loader: String,
    contents: Vec<ContentSelection>,
    profile: Option<Value>,
) -> Result<ContentReport, String> {
    let _install_guard = crate::minecraft::INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Spielinstallation oder ein Start läuft bereits. Bitte warten.")?;
    crate::launch::ensure_stopped()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("minecraft");
    let progress = Arc::new(move |event: ContentProgress| {
        let _ = app.emit("content-install-progress", event);
    });
    install_with_progress(
        &root,
        &instance_id,
        &game_version,
        &loader,
        contents,
        profile,
        progress,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fixture {
        versions: BTreeMap<String, ApiVersion>,
        projects: BTreeMap<String, ApiProject>,
        calls: usize,
    }
    impl Metadata for Fixture {
        async fn version(&mut self, id: &str) -> Result<ApiVersion, String> {
            self.calls += 1;
            self.versions
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Missing fixture {id}"))
        }
        async fn project(&mut self, id: &str) -> Result<ApiProject, String> {
            self.calls += 1;
            self.projects
                .get(id)
                .cloned()
                .ok_or_else(|| format!("Missing project {id}"))
        }
        async fn versions(
            &mut self,
            project: &str,
            _game: &str,
        ) -> Result<Vec<ApiVersion>, String> {
            self.calls += 1;
            Ok(self
                .versions
                .values()
                .filter(|v| v.project_id == project)
                .cloned()
                .collect())
        }
    }

    fn version(id: &str, project: &str, dependencies: serde_json::Value) -> ApiVersion {
        serde_json::from_value(json!({"id":id,"project_id":project,"version_number":id,"version_type":"release","game_versions":["1.21.1"],"loaders":["fabric"],"date_published":"2026-01-01T00:00:00Z","files":[{"filename":format!("{id}.jar"),"url":format!("https://cdn.modrinth.com/data/{project}/versions/{id}/{id}.jar"),"hashes":{"sha512":format!("{:x}",Sha512::digest(b"jar"))},"primary":true,"size":3}],"dependencies":dependencies})).unwrap()
    }

    fn project(id: &str) -> ApiProject {
        serde_json::from_value(
            json!({"id":id,"title":id,"project_type":"mod","client_side":"required"}),
        )
        .unwrap()
    }

    fn selection(version: &ApiVersion) -> ContentSelection {
        ContentSelection {
            id: if version.source == "modrinth" {
                format!("item-{}", version.project_id)
            } else {
                format!("item-{}-{}", version.source, version.project_id)
            },
            project_id: version.project_id.clone(),
            version_id: version.id.clone(),
            name: version.project_id.clone(),
            version: version.version_number.clone(),
            filename: version.files[0].filename.clone(),
            icon_url: None,
            kind: "mod".into(),
            enabled: true,
            sha512: version.files[0].hashes.get("sha512").cloned(),
            sha1: version.files[0].hashes.get("sha1").cloned(),
            source: version.source.clone(),
            status: "planned".into(),
            dependency: false,
        }
    }

    fn fixture(versions: Vec<ApiVersion>) -> Fixture {
        let mut result = Fixture::default();
        for version in versions {
            result
                .projects
                .insert(version.project_id.clone(), project(&version.project_id));
            result.versions.insert(version.id.clone(), version);
        }
        result
    }

    struct MixedFixture {
        versions: BTreeMap<String, ApiVersion>,
        projects: BTreeMap<String, ApiProject>,
    }
    impl SourcesMetadata for MixedFixture {
        async fn version_for(
            &mut self,
            source: &str,
            _project: Option<&str>,
            id: &str,
        ) -> Result<ApiVersion, String> {
            self.versions
                .get(&project_key(source, id))
                .cloned()
                .ok_or_else(|| format!("Missing {source}:{id}"))
        }
        async fn project_for(&mut self, source: &str, id: &str) -> Result<ApiProject, String> {
            self.projects
                .get(&project_key(source, id))
                .cloned()
                .ok_or_else(|| format!("Missing {source}:{id}"))
        }
        async fn versions_for(
            &mut self,
            source: &str,
            project: &str,
            _game: &str,
            _loader: &str,
        ) -> Result<Vec<ApiVersion>, String> {
            Ok(self
                .versions
                .values()
                .filter(|version| version.source == source && version.project_id == project)
                .cloned()
                .collect())
        }
    }
    fn mixed_fixture(versions: Vec<ApiVersion>) -> MixedFixture {
        let mut fixture = MixedFixture {
            versions: BTreeMap::new(),
            projects: BTreeMap::new(),
        };
        for version in versions {
            fixture.projects.insert(
                project_key(&version.source, &version.project_id),
                project(&version.project_id),
            );
            fixture
                .versions
                .insert(project_key(&version.source, &version.id), version);
        }
        fixture
    }
    fn cf_version(id: u64, project: u64, payload: &[u8], dependencies: Value) -> ApiVersion {
        let raw:crate::curseforge::CfFile=serde_json::from_value(json!({"id":id,"modId":project,"gameId":432,"fileName":format!("cf-{id}.jar"),"fileLength":payload.len(),"isAvailable":true,"downloadUrl":"https://edge.forgecdn.net/files/9999/1/fixture.jar","gameVersions":["1.21.1","Fabric"],"hashes":[{"value":format!("{:x}",Sha1::digest(payload)),"algo":1}],"dependencies":dependencies,"releaseType":1,"fileDate":"2026-11-05T00:00:00Z","fileStatus":4})).unwrap();
        curseforge_version(raw).unwrap()
    }

    #[test]
    fn mixed_sources_keep_equal_project_and_version_ids_in_separate_dependency_graphs() {
        let mut mr = version(
            "900",
            "123",
            json!([{"project_id":"456","dependency_type":"required"}]),
        );
        mr.files[0]
            .hashes
            .insert("sha512".into(), format!("{:x}", Sha512::digest(b"mr-root")));
        let mut mr_dep = version("901", "456", json!([]));
        mr_dep.files[0].hashes.insert(
            "sha512".into(),
            format!("{:x}", Sha512::digest(b"mr-dependency")),
        );
        let cf = cf_version(
            900,
            123,
            b"cf-root",
            json!([{"modId":456,"relationType":3}]),
        );
        let cf_dep = cf_version(901, 456, b"cf-dependency", json!([]));
        let roots = vec![selection(&mr), selection(&cf)];
        validate_selections(&roots).unwrap();
        let mut noncanonical = roots[1].clone();
        noncanonical.project_id = "0123".into();
        assert!(validate_selections(&[noncanonical]).is_err());
        let mut fixture = mixed_fixture(vec![mr, mr_dep, cf, cf_dep]);
        let resolved = runtime()
            .block_on(resolve(&mut fixture, "1.21.1", "fabric", &roots))
            .unwrap();
        assert_eq!(resolved.len(), 4);
        assert!(resolved.iter().any(|item| item.selection.id == "dep-456"));
        assert!(resolved
            .iter()
            .any(|item| item.selection.id == "dep-curseforge-456"));
        assert_eq!(
            resolved
                .iter()
                .filter(|item| item.selection.project_id == "123")
                .count(),
            2
        );
    }

    #[test]
    fn curseforge_required_versions_conflict_and_distribution_optout_stops_resolution() {
        let a = cf_version(
            100,
            10,
            b"a",
            json!([{"modId":30,"fileId":300,"relationType":3}]),
        );
        let b = cf_version(
            200,
            20,
            b"b",
            json!([{"modId":30,"fileId":301,"relationType":3}]),
        );
        let dep1 = cf_version(300, 30, b"dep-a", json!([]));
        let dep2 = cf_version(301, 30, b"dep-b", json!([]));
        let mut fixture = mixed_fixture(vec![a.clone(), b.clone(), dep1, dep2]);
        assert!(runtime()
            .block_on(resolve(
                &mut fixture,
                "1.21.1",
                "fabric",
                &[selection(&a), selection(&b)]
            ))
            .unwrap_err()
            .contains("Versionskonflikt"));
        let mut fixture = mixed_fixture(vec![a.clone()]);
        fixture
            .projects
            .get_mut("curseforge:10")
            .unwrap()
            .distribution_allowed = false;
        assert!(runtime()
            .block_on(resolve(&mut fixture, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap_err()
            .contains("Verteilung"));
    }

    #[test]
    fn curseforge_required_dependency_can_use_compatible_beta_when_no_release_exists() {
        let root = cf_version(
            100,
            10,
            b"root-release",
            json!([{"modId":20,"relationType":3}]),
        );
        let mut dependency = cf_version(200, 20, b"dependency-beta", json!([]));
        dependency.version_type = "beta".into();
        let mut fixture = mixed_fixture(vec![root.clone(), dependency]);
        let resolved = runtime()
            .block_on(resolve(
                &mut fixture,
                "1.21.1",
                "fabric",
                &[selection(&root)],
            ))
            .unwrap();
        let required = resolved
            .iter()
            .find(|item| item.selection.project_id == "20")
            .unwrap();
        assert_eq!(required.version.version_type, "beta");
        assert!(required.selection.dependency);
    }

    #[test]
    fn curseforge_sha1_is_provider_proof_and_computed_sha512_is_local_only() {
        let root = temp_root();
        let payload = b"genuine curseforge bytes";
        let version = cf_version(100, 10, payload, json!([]));
        let file = &version.files[0];
        let target = root.join("cf.jar");
        fs::write(&target, payload).unwrap();
        let computed = verified_curseforge_bytes(&target, file, None)
            .unwrap()
            .unwrap();
        assert_eq!(computed, format!("{:x}", Sha512::digest(payload)));
        let mut installed = selection(&version);
        installed.sha512 = Some(computed.clone());
        let planned = selection(&version);
        assert_eq!(
            fingerprint(&[installed.clone()]).unwrap(),
            fingerprint(&[planned]).unwrap()
        );
        let mut changed = installed.clone();
        changed.sha1 = Some("f".repeat(40));
        assert_ne!(
            fingerprint(&[changed.clone()]).unwrap(),
            fingerprint(&[installed]).unwrap()
        );
        assert!(selected_file(&version, Some(&changed), "mod").is_err());
        assert!(verified_curseforge_bytes(&target, file, Some(&"f".repeat(128))).is_err());
        fs::write(&target, vec![b'!'; payload.len()]).unwrap();
        assert!(verified_curseforge_bytes(&target, file, None)
            .unwrap()
            .is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_mod_bytes_from_different_sources_are_refused_without_guessing_identity() {
        let mr = version("900", "123", json!([]));
        let cf = cf_version(900, 123, b"jar", json!([]));
        let mut fixture = mixed_fixture(vec![mr.clone(), cf.clone()]);
        let mut resolved = runtime()
            .block_on(resolve(
                &mut fixture,
                "1.21.1",
                "fabric",
                &[selection(&mr), selection(&cf)],
            ))
            .unwrap();
        for item in &mut resolved {
            if item.selection.source == "curseforge" {
                item.selection.sha512 = Some(format!("{:x}", Sha512::digest(b"jar")));
            }
        }
        assert!(distinct_mod_binaries(&resolved)
            .unwrap_err()
            .contains("dieselben Moddateibytes"));
    }

    #[test]
    fn curseforge_work_files_are_not_a_persistent_download_cache() {
        let root = temp_root();
        let directory;
        {
            let work = ContentWork::new(&root, true).unwrap();
            directory = work.directory.clone().unwrap();
            let target = work.file(0).unwrap();
            fs::write(&target, b"temporary").unwrap();
            assert!(target.exists());
        }
        assert!(!directory.exists());
        assert!(!root.join("content-cache").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_legacy_modrinth_lock_keeps_original_digest_without_migration() {
        // Exact pins, hashes and desired digest from the user's original v1 content lock.
        // No provider metadata request or account/key access is needed to read it.
        let root = temp_root();
        let instance = "instance-5e1294ba-2ebe-4719-ba92-d2b6fd353b0a";
        let sodium="8614a374593698069a80ac74de8cd9c28f3928d16819a0e0d6320b538e823f7e54e8be06d5ea5a9c268beb0e02073209d1f831a02dede851588b972367680853";
        let extra="2587475562472b7e5cdc8c2b4409de498815402156524242acbad9672a0c348288b9b3328ec82b9300593d1a02eb71022704742889e5b33b755f75709cf16827";
        let desired="5a5d6cefa09add14dde4a33e7aad306bc0532639b66119d8067d53aadb4aa4df81837fbf5040eced3edf8751a092346e799946fff6ddedd6095537d89564c504";
        let legacy = json!({"format":"syncmc-content-lock","version":1,"instanceId":instance,"gameVersion":"1.21.1","loader":"fabric","desiredSha512":desired,"contents":[
            {"id":"dep-AANobbMI","projectId":"AANobbMI","versionId":"SMxNOGZ6","name":"Sodium","version":"mc1.21.1-0.8.13-fabric","filename":"sodium-fabric-0.8.13+mc1.21.1.jar","kind":"mod","enabled":true,"sha512":sodium,"source":"modrinth","status":"downloaded","dependency":true},
            {"id":"fefa7252-9567-4dfc-8da9-5e561ad4c4a2","projectId":"PtjYWJkn","versionId":"CmiqMBZT","name":"Sodium Extra","version":"mc1.21.1-0.9.5+fabric","filename":"sodium-extra-fabric-0.9.5+mc1.21.1.jar","kind":"mod","enabled":true,"sha512":extra,"source":"modrinth","status":"downloaded"}],"files":[
                {"path":"mods/sodium-fabric-0.8.13+mc1.21.1.jar","sha512":sodium,"size":1574609},
                {"path":"mods/sodium-extra-fabric-0.9.5+mc1.21.1.jar","sha512":extra,"size":492939}]});
        let path = prepare_file(&root, &lock_relative(instance).unwrap()).unwrap();
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &original).unwrap();
        let lock = read_lock(&root, instance).unwrap().unwrap();
        assert_eq!(fingerprint(&lock.contents).unwrap(), desired);
        assert_eq!(fs::read(&path).unwrap(), original);
        // Verification reaches the actual byte-proof check, rather than treating v1 as corrupt.
        assert!(
            verify_installation(&root, instance, "1.21.1", "fabric", &lock.contents)
                .unwrap_err()
                .contains("fehlt oder ist beschädigt")
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn resolves_recursive_dependencies_and_cycles_once() {
        let a = version(
            "a1",
            "a",
            json!([{"version_id":"b1","project_id":"b","dependency_type":"required"}]),
        );
        let b = version(
            "b1",
            "b",
            json!([{"version_id":"a1","project_id":"a","dependency_type":"required"}]),
        );
        let mut repo = fixture(vec![a.clone(), b]);
        let result = runtime()
            .block_on(resolve(&mut repo, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.iter().filter(|r| r.selection.dependency).count(), 1);
        assert!(
            repo.calls < 10,
            "cycle should terminate before refetching versions"
        );
    }

    #[test]
    fn exact_dependency_conflicts_with_explicit_pin() {
        let a = version(
            "a1",
            "a",
            json!([{"version_id":"b1","project_id":"b","dependency_type":"required"}]),
        );
        let b1 = version("b1", "b", json!([]));
        let b2 = version("b2", "b", json!([]));
        let mut repo = fixture(vec![a.clone(), b1, b2.clone()]);
        assert!(runtime()
            .block_on(resolve(
                &mut repo,
                "1.21.1",
                "fabric",
                &[selection(&a), selection(&b2)]
            ))
            .unwrap_err()
            .contains("Versionskonflikt"));
    }

    #[test]
    fn project_dependency_prefers_release_and_preserves_saved_pin() {
        let a = version(
            "a1",
            "a",
            json!([{"project_id":"b","dependency_type":"required"}]),
        );
        let b1 = version("b1", "b", json!([]));
        let mut b2 = version("b2", "b", json!([]));
        b2.version_type = "beta".into();
        b2.date_published = "2026-11-05".into();
        let mut repo = fixture(vec![a.clone(), b1, b2.clone()]);
        let result = runtime()
            .block_on(resolve(&mut repo, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap();
        assert_eq!(
            result
                .iter()
                .find(|r| r.selection.project_id == "b")
                .unwrap()
                .version
                .id,
            "b1"
        );
        let mut saved = selection(&b2);
        saved.dependency = true;
        let result = runtime()
            .block_on(resolve(
                &mut repo,
                "1.21.1",
                "fabric",
                &[selection(&a), saved],
            ))
            .unwrap();
        assert_eq!(
            result
                .iter()
                .find(|r| r.selection.project_id == "b")
                .unwrap()
                .version
                .id,
            "b2"
        );
    }

    #[test]
    fn changed_root_updates_automatic_pin_and_exact_constraints_win_before_project_choice() {
        let a = version(
            "a2",
            "a",
            json!([{"version_id":"b2","project_id":"b","dependency_type":"required"}]),
        );
        let b1 = version("b1", "b", json!([]));
        let b2 = version("b2", "b", json!([]));
        let mut automatic = selection(&b1);
        automatic.dependency = true;
        let mut repo = fixture(vec![a.clone(), b1.clone(), b2.clone()]);
        let result = runtime()
            .block_on(resolve(
                &mut repo,
                "1.21.1",
                "fabric",
                &[selection(&a), automatic.clone()],
            ))
            .unwrap();
        assert_eq!(
            result
                .iter()
                .find(|r| r.selection.project_id == "b")
                .unwrap()
                .version
                .id,
            "b2"
        );
        assert!(
            result
                .iter()
                .find(|r| r.selection.project_id == "b")
                .unwrap()
                .selection
                .dependency
        );

        let a = version(
            "a1",
            "a",
            json!([{"project_id":"b","dependency_type":"required"}]),
        );
        let c = version(
            "c1",
            "c",
            json!([{"project_id":"b","version_id":"b2","dependency_type":"required"}]),
        );
        let mut repo = fixture(vec![a.clone(), c.clone(), b1, b2]);
        let result = runtime()
            .block_on(resolve(
                &mut repo,
                "1.21.1",
                "fabric",
                &[selection(&a), selection(&c), automatic],
            ))
            .unwrap();
        assert_eq!(
            result
                .iter()
                .find(|r| r.selection.project_id == "b")
                .unwrap()
                .version
                .id,
            "b2"
        );
    }

    #[test]
    fn late_exact_constraint_restarts_auto_choice_and_discards_old_derived_dependencies() {
        let a = version(
            "a1",
            "a",
            json!([{"project_id":"b","dependency_type":"required"},{"project_id":"c","dependency_type":"required"}]),
        );
        let mut b1 = version(
            "b1",
            "b",
            json!([{"project_id":"d","version_id":"d1","dependency_type":"required"}]),
        );
        b1.date_published = "2026-11-05".into();
        let mut b2 = version("b2", "b", json!([]));
        b2.date_published = "2026-11-04".into();
        let c = version(
            "c1",
            "c",
            json!([{"project_id":"b","version_id":"b2","dependency_type":"required"}]),
        );
        let d = version("d1", "d", json!([]));
        let mut saved_b1 = selection(&b1);
        saved_b1.dependency = true;
        let mut saved_d = selection(&d);
        saved_d.dependency = true;
        let mut fixture = fixture(vec![a.clone(), b1, b2, c, d]);
        let resolved = runtime()
            .block_on(resolve(
                &mut fixture,
                "1.21.1",
                "fabric",
                &[selection(&a), saved_b1, saved_d],
            ))
            .unwrap();
        assert_eq!(resolved.len(), 3);
        assert_eq!(
            resolved
                .iter()
                .find(|item| item.selection.project_id == "b")
                .unwrap()
                .version
                .id,
            "b2"
        );
        assert!(!resolved.iter().any(|item| item.selection.project_id == "d"));

        let a = cf_version(
            100,
            1,
            b"a",
            json!([{"modId":2,"relationType":3},{"modId":3,"relationType":3}]),
        );
        let mut b1 = cf_version(
            200,
            2,
            b"b1",
            json!([{"modId":4,"fileId":400,"relationType":3}]),
        );
        b1.date_published = "2026-11-05".into();
        let mut b2 = cf_version(201, 2, b"b2", json!([]));
        b2.date_published = "2026-11-04".into();
        let c = cf_version(
            300,
            3,
            b"c",
            json!([{"modId":2,"fileId":201,"relationType":3}]),
        );
        let d = cf_version(400, 4, b"d", json!([]));
        let mut fixture = mixed_fixture(vec![a.clone(), b1, b2, c, d]);
        let resolved = runtime()
            .block_on(resolve(&mut fixture, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap();
        assert_eq!(resolved.len(), 3);
        assert_eq!(
            resolved
                .iter()
                .find(|item| item.selection.project_id == "2")
                .unwrap()
                .version
                .id,
            "201"
        );
        assert!(!resolved.iter().any(|item| item.selection.project_id == "4"));
    }

    #[test]
    fn repeated_automatic_preferences_stop_without_unbounded_backtracking() {
        let a = version(
            "a1",
            "a",
            json!([{"project_id":"b","dependency_type":"required"},{"project_id":"c","dependency_type":"required"}]),
        );
        let mut b1 = version("b1", "b", json!([]));
        b1.date_published = "2026-11-05".into();
        let mut b2 = version(
            "b2",
            "b",
            json!([{"project_id":"c","version_id":"c2","dependency_type":"required"}]),
        );
        b2.date_published = "2026-11-04".into();
        let mut c1 = version(
            "c1",
            "c",
            json!([{"project_id":"b","version_id":"b2","dependency_type":"required"}]),
        );
        c1.date_published = "2026-11-05".into();
        let mut c2 = version(
            "c2",
            "c",
            json!([{"project_id":"b","version_id":"b1","dependency_type":"required"}]),
        );
        c2.date_published = "2026-11-04".into();
        let mut fixture = fixture(vec![a.clone(), b1, b2, c1, c2]);
        assert!(runtime()
            .block_on(resolve(&mut fixture, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap_err()
            .contains("wechselnde Versionen"));
        assert!(fixture.calls < 100);
    }

    #[test]
    fn blocks_declared_incompatible_versions_and_ignores_optional() {
        let a = version(
            "a1",
            "a",
            json!([{"project_id":"b","dependency_type":"incompatible"},{"project_id":"missing","dependency_type":"optional"}]),
        );
        let b = version("b1", "b", json!([]));
        let mut repo = fixture(vec![a.clone(), b.clone()]);
        assert_eq!(
            runtime()
                .block_on(resolve(&mut repo, "1.21.1", "fabric", &[selection(&a)]))
                .unwrap()
                .len(),
            1
        );
        assert!(runtime()
            .block_on(resolve(
                &mut repo,
                "1.21.1",
                "fabric",
                &[selection(&a), selection(&b)]
            ))
            .unwrap_err()
            .contains("nicht kompatibel"));
    }

    #[test]
    fn rejects_wrong_project_game_loader_and_server_only_artifacts() {
        let a = version("a1", "a", json!([]));
        let mut wrong = selection(&a);
        wrong.project_id = "b".into();
        assert!(runtime()
            .block_on(resolve(
                &mut fixture(vec![a.clone()]),
                "1.21.1",
                "fabric",
                &[wrong]
            ))
            .is_err());
        assert!(runtime()
            .block_on(resolve(
                &mut fixture(vec![a.clone()]),
                "1.20.1",
                "fabric",
                &[selection(&a)]
            ))
            .is_err());
        let mut forge = a.clone();
        forge.loaders = vec!["forge".into()];
        assert!(runtime()
            .block_on(resolve(
                &mut fixture(vec![forge.clone()]),
                "1.21.1",
                "fabric",
                &[selection(&forge)]
            ))
            .is_err());
        let mut repo = fixture(vec![a.clone()]);
        repo.projects.get_mut("a").unwrap().client_side = Some("unsupported".into());
        assert!(runtime()
            .block_on(resolve(&mut repo, "1.21.1", "fabric", &[selection(&a)]))
            .unwrap_err()
            .contains("Server-Mod"));
    }

    #[test]
    fn rejects_changed_file_hash_and_demo_selection() {
        let a = version("a1", "a", json!([]));
        let mut selected = selection(&a);
        selected.sha512 = Some("f".repeat(128));
        assert!(runtime()
            .block_on(resolve(
                &mut fixture(vec![a.clone()]),
                "1.21.1",
                "fabric",
                &[selected]
            ))
            .unwrap_err()
            .contains("Prüfsumme"));
        let mut demo = selection(&a);
        demo.version_id = "demo-a".into();
        assert!(runtime()
            .block_on(resolve(
                &mut Fixture::default(),
                "1.21.1",
                "fabric",
                &[demo]
            ))
            .unwrap_err()
            .contains("Beispiel"));
    }

    #[test]
    fn rejects_unsafe_names_paths_and_download_hosts() {
        for name in [
            "../evil.jar",
            "evil\\x.jar",
            "CON.jar",
            "aux.zip",
            "foo.jar ",
            "..",
            "foo\n.jar",
        ] {
            assert!(filename(name).is_err(), "{name}");
        }
        for path in ["../mods/a.jar", "/mods/a.jar", "mods/../a.jar"] {
            assert!(checked_relative(Path::new(path)).is_err(), "{path}");
        }
        #[cfg(not(target_os = "windows"))]
        assert!(checked_relative(Path::new("mods\\a.jar")).is_err());
        checked_relative(
            &PathBuf::from("instances")
                .join("test")
                .join("mods")
                .join("a.jar"),
        )
        .unwrap();
        for url in [
            "http://cdn.modrinth.com/data/a.jar",
            "https://cdn.modrinth.com.evil.test/data/a.jar",
            "https://u:p@cdn.modrinth.com/data/a.jar",
            "https://cdn.modrinth.com:444/data/a.jar",
            "https://example.com/a.jar",
        ] {
            assert!(cdn_url(url).is_err(), "{url}");
        }
    }

    fn temp_root() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "syncmc-content-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn verifies_bytes_and_detects_changed_profile_or_unmanaged_jar() {
        let root = temp_root();
        let a = version("a1", "a", json!([]));
        let mut selected = selection(&a);
        selected.status = "downloaded".into();
        let file = LockedFile {
            path: "mods/a1.jar".into(),
            sha512: selected.sha512.clone().unwrap(),
            size: 3,
        };
        let target = prepare_file(&root, Path::new("instances/test/mods/a1.jar")).unwrap();
        fs::write(&target, b"jar").unwrap();
        let lock = ContentLock {
            format: "syncmc-content-lock".into(),
            version: 1,
            instance_id: "test".into(),
            game_version: "1.21.1".into(),
            loader: "fabric".into(),
            desired_sha512: fingerprint(&[selected.clone()]).unwrap(),
            contents: vec![selected.clone()],
            files: vec![file],
            profile: None,
        };
        write_lock(&root, "test", &lock).unwrap();
        verify_installation(&root, "test", "1.21.1", "fabric", &[selected.clone()]).unwrap();
        selected.enabled = false;
        assert!(
            verify_installation(&root, "test", "1.21.1", "fabric", &[selected.clone()])
                .unwrap_err()
                .contains("verändert")
        );
        selected.enabled = true;
        fs::write(&target, b"bad").unwrap();
        assert!(
            verify_installation(&root, "test", "1.21.1", "fabric", &[selected.clone()])
                .unwrap_err()
                .contains("beschädigt")
        );
        fs::write(&target, b"jar").unwrap();
        fs::write(target.with_file_name("foreign.jar"), b"user").unwrap();
        assert!(
            verify_installation(&root, "test", "1.21.1", "fabric", &[selected])
                .unwrap_err()
                .contains("nicht im Profil")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cosmetic_ui_fields_do_not_invalidate_installation() {
        let a = version("a1", "a", json!([]));
        let selected = selection(&a);
        let mut renamed = selected.clone();
        renamed.name = "Custom label".into();
        renamed.status = "downloaded".into();
        renamed.id = "changed-ui-id".into();
        assert_eq!(
            fingerprint(&[selected]).unwrap(),
            fingerprint(&[renamed]).unwrap()
        );
    }

    #[test]
    fn backup_keeps_old_pins_and_restores_verified_mods_world_and_rescue() {
        let root = temp_root();
        let first = version("a1", "a", json!([]));
        let second = version("a2", "a", json!([]));
        let old = selection(&first);
        let updated = selection(&second);
        let mut old_profile =
            crate::backup::default_profile("test", "1.21.1", "fabric", &[old.clone()]);
        old_profile["name"] = json!("Vor dem Update");
        let mut candidate =
            crate::backup::default_profile("test", "1.21.1", "fabric", &[updated.clone()]);
        candidate["name"] = json!("Aktueller Instanzname");
        candidate["cover"] = json!("redwood");
        candidate["servers"] =
            json!([{"id":"new-server","name":"Neuer Server","address":"play.example.org"}]);
        let old_lock = ContentLock {
            format: "syncmc-content-lock".into(),
            version: 1,
            instance_id: "test".into(),
            game_version: "1.21.1".into(),
            loader: "fabric".into(),
            desired_sha512: fingerprint(&[old.clone()]).unwrap(),
            contents: vec![old.clone()],
            files: vec![LockedFile {
                path: content_path(&old).unwrap(),
                sha512: old.sha512.clone().unwrap(),
                size: 3,
            }],
            profile: Some(old_profile.clone()),
        };
        let old_mod = prepare_file(
            &root,
            &instance_relative("test")
                .unwrap()
                .join(content_path(&old).unwrap()),
        )
        .unwrap();
        fs::write(&old_mod, b"jar").unwrap();
        write_lock(&root, "test", &old_lock).unwrap();
        let mut wrong_id = candidate.clone();
        wrong_id["id"] = json!("other-instance");
        assert!(profile_for_snapshot(&root, "test", Some(wrong_id)).is_err());
        let world = prepare_file(&root, Path::new("instances/test/saves/World/level.dat")).unwrap();
        fs::write(&world, b"old-world").unwrap();
        let config = prepare_file(&root, Path::new("instances/test/config/mod.json")).unwrap();
        fs::write(&config, b"old-config").unwrap();
        let snapshot = profile_for_snapshot(&root, "test", Some(candidate.clone())).unwrap();
        assert_eq!(snapshot["contents"][0]["versionId"], "a1");
        assert_eq!(snapshot["name"], "Aktueller Instanzname");
        assert_eq!(snapshot["cover"], "redwood");
        assert_eq!(snapshot["servers"][0]["id"], "new-server");
        let backup =
            crate::backup::create_at(&root, "test", snapshot, "before-content-change").unwrap();
        assert!(backup.verified_content);
        let new_mod = prepare_file(
            &root,
            &instance_relative("test")
                .unwrap()
                .join(content_path(&updated).unwrap()),
        )
        .unwrap();
        fs::write(&new_mod, b"jar").unwrap();
        fs::remove_file(&old_mod).unwrap();
        fs::write(&world, b"new-world").unwrap();
        fs::write(&config, b"new-config").unwrap();
        let new_lock = ContentLock {
            desired_sha512: fingerprint(&[updated.clone()]).unwrap(),
            contents: vec![updated.clone()],
            files: vec![LockedFile {
                path: content_path(&updated).unwrap(),
                sha512: updated.sha512.clone().unwrap(),
                size: 3,
            }],
            profile: Some(candidate),
            ..old_lock
        };
        write_lock(&root, "test", &new_lock).unwrap();
        let restored = crate::backup::restore_at(&root, "test", &backup.id).unwrap();
        assert_eq!(restored.profile["contents"][0]["versionId"], "a1");
        assert!(old_mod.exists());
        assert!(!new_mod.exists());
        assert_eq!(fs::read(&world).unwrap(), b"old-world");
        assert_eq!(fs::read(&config).unwrap(), b"old-config");
        verify_installation(&root, "test", "1.21.1", "fabric", &restored.contents).unwrap();
        let rescue = crate::backup::restore_at(&root, "test", &restored.rescue_backup.id).unwrap();
        assert_eq!(rescue.profile["contents"][0]["versionId"], "a2");
        assert!(new_mod.exists());
        assert_eq!(fs::read(&world).unwrap(), b"new-world");
        verify_installation(&root, "test", "1.21.1", "fabric", &rescue.contents).unwrap();
        // Even if all backup file hashes match their manifest, its semantic content proof
        // must still agree with the profile before the live directory can be replaced.
        let saved_lock = root
            .join("backups/test")
            .join(&backup.id)
            .join("data/content-lock.json");
        let mut inconsistent: Value =
            serde_json::from_slice(&fs::read(&saved_lock).unwrap()).unwrap();
        inconsistent["contents"][0]["versionId"] = json!("ax");
        let changed = serde_json::to_vec_pretty(&inconsistent).unwrap();
        fs::write(&saved_lock, &changed).unwrap();
        let backup_manifest = root
            .join("backups/test")
            .join(&backup.id)
            .join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&backup_manifest).unwrap()).unwrap();
        let changed_entry = manifest["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == "content-lock.json")
            .unwrap();
        let previous_size = changed_entry["size"].as_u64().unwrap();
        changed_entry["size"] = json!(changed.len());
        changed_entry["sha512"] = json!(format!("{:x}", Sha512::digest(&changed)));
        manifest["summary"]["totalBytes"] = json!(
            manifest["summary"]["totalBytes"].as_u64().unwrap() - previous_size
                + changed.len() as u64
        );
        fs::write(
            backup_manifest,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(crate::backup::restore_at(&root, "test", &backup.id).is_err());
        assert_eq!(fs::read(&world).unwrap(), b"new-world");
        assert!(new_mod.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_transaction_blocks_launch_and_preserves_owned_paths() {
        let root = temp_root();
        let pending = PendingContent {
            format: "syncmc-content-pending".into(),
            version: 1,
            instance_id: "test".into(),
            files: vec![LockedFile {
                path: "mods/partially-copied.jar".into(),
                sha512: format!("{:x}", Sha512::digest(b"partial")),
                size: 7,
            }],
        };
        write_json(&root, &pending_relative("test").unwrap(), &pending).unwrap();
        assert_eq!(
            read_pending(&root, "test").unwrap()[0].path,
            "mods/partially-copied.jar"
        );
        assert!(verify_installation(&root, "test", "1.21.1", "fabric", &[])
            .unwrap_err()
            .contains("unterbrochen"));
        let mut tampered = pending;
        tampered.files[0].path = "../foreign.jar".into();
        write_json(&root, &pending_relative("test").unwrap(), &tampered).unwrap();
        assert!(read_pending(&root, "test").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Downloads real Modrinth artifacts; explicitly run with network access"]
    fn real_modrinth_installation() {
        runtime().block_on(async {
            let root = std::env::var_os("SYNCMC_FABRIC_TEST_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/private/tmp/syncmc-fabric-smoke"));
            let game = "1.21.1";
            let instance = "fabric-smoke";
            let mut repo = ModrinthMetadata::default();
            // Sodium Extra declares actual required Modrinth dependencies, unlike a standalone JAR.
            let project_id = "PtjYWJkn";
            let project = repo.project(project_id).await.unwrap();
            let mut versions = repo.versions(project_id, game).await.unwrap();
            versions.sort_by(|a,b| b.date_published.cmp(&a.date_published));
            let version = versions.into_iter().find(|v| v.version_type == "release" && compatible(v,&project,game,"fabric","mod").is_ok() && v.dependencies.iter().any(|d| d.dependency_type == "required")).expect("Smoke project needs a compatible release with a required dependency");
            let file = selected_file(&version,None,"mod").unwrap();
            let mut selected=selection(&version);selected.filename=file.filename;selected.sha512=file.hashes.get("sha512").cloned();
            let original=selected.clone();
            let first=install_at(&root,instance,game,"fabric",vec![selected]).await.unwrap();
            assert!(first.dependency_count>0,"Smoke must install real required dependencies");
            verify_installation(&root,instance,game,"fabric",&first.contents).unwrap();
            eprintln!("Modrinth: {} verified files, {} required dependencies",first.files_verified,first.dependency_count);

            let target=root.join("instances").join(instance).join(content_path(first.contents.iter().find(|s| !s.dependency).unwrap()).unwrap());
            fs::write(&target,b"deliberately corrupted smoke artifact").unwrap();
            assert!(verify_installation(&root,instance,game,"fabric",&first.contents).is_err());
            let repaired=install_at(&root,instance,game,"fabric",first.contents.clone()).await.unwrap();
            verify_installation(&root,instance,game,"fabric",&repaired.contents).unwrap();

            let mut disabled=repaired.contents.clone();disabled.iter_mut().find(|s| !s.dependency).unwrap().enabled=false;
            let disabled=install_at(&root,instance,game,"fabric",disabled).await.unwrap();
            assert_eq!(disabled.files_verified,0);
            assert!(!target.exists(),"Disabled root and orphaned dependencies must be removed");
            verify_installation(&root,instance,game,"fabric",&disabled.contents).unwrap();
            let removed=install_at(&root,instance,game,"fabric",vec![]).await.unwrap();
            assert!(removed.contents.is_empty());
            verify_installation(&root,instance,game,"fabric",&[]).unwrap();

            let restored=install_at(&root,instance,game,"fabric",vec![original]).await.unwrap();
            let old=read_lock(&root,instance).unwrap().unwrap();
            let partial=LockedFile{path:"mods/interrupted-smoke.jar".into(),sha512:format!("{:x}",Sha512::digest(b"partial")),size:7};
            let partial_path=prepare_file(&root,&instance_relative(instance).unwrap().join(&partial.path)).unwrap();
            fs::write(&partial_path,b"partial").unwrap();
            let pending=PendingContent{format:"syncmc-content-pending".into(),version:1,instance_id:instance.into(),files:old.files.into_iter().chain([partial]).collect()};
            write_json(&root,&pending_relative(instance).unwrap(),&pending).unwrap();
            assert!(verify_installation(&root,instance,game,"fabric",&restored.contents).unwrap_err().contains("unterbrochen"));
            let recovered=install_at(&root,instance,game,"fabric",restored.contents).await.unwrap();
            assert!(!partial_path.exists(),"Interrupted app-owned artifact must be cleaned up on retry");
            verify_installation(&root,instance,game,"fabric",&recovered.contents).unwrap();
            let receipt=json!({"gameVersion":game,"loader":"fabric","instanceId":instance,"contents":recovered.contents,"filesVerified":recovered.files_verified,"dependencyCount":recovered.dependency_count,"checks":{"sha512":"passed","requiredDependencies":"passed","corruptFileRepair":"passed","disableAndOrphanCleanup":"passed","removeAll":"passed","interruptedCommitRecovery":"passed"}});
            fs::write(root.join("content-smoke-results.json"),serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_content_directory() {
        let root = temp_root();
        let outside = temp_root();
        fs::create_dir_all(root.join("instances/test")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("instances/test/mods")).unwrap();
        assert!(prepare_file(&root, Path::new("instances/test/mods/a.jar")).is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
