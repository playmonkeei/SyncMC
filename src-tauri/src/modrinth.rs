use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;
use std::time::Duration;

const API_BASE: &str = "https://api.modrinth.com/v2";
const MAX_PROJECT_LINK_BYTES: usize = 8 * 1024 * 1024;
static HTTP_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

#[derive(Debug, Serialize)]
pub struct ProjectLinks {
    pub project_id: String,
    pub source_url: Option<String>,
}

#[derive(Deserialize)]
struct ProjectDetails {
    id: String,
    #[serde(default)]
    source_url: Option<String>,
}

fn project_link_ids(ids: Vec<String>) -> Result<BTreeSet<String>, String> {
    if ids.len() > 24 {
        return Err(
            "Es können höchstens 24 Modrinth-Projektlinks gleichzeitig geladen werden.".into(),
        );
    }
    let mut validated = BTreeSet::new();
    for id in ids {
        if id.is_empty()
            || id.len() > 100
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Eine Modrinth-Projekt-ID ist ungültig.".into());
        }
        validated.insert(id);
    }
    Ok(validated)
}

fn safe_source_url(value: &str) -> Option<String> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return None;
    }
    let mut url = reqwest::Url::parse(value).ok()?;
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

fn project_links(
    details: Vec<ProjectDetails>,
    requested: &BTreeSet<String>,
) -> Result<Vec<ProjectLinks>, String> {
    if details.len() > requested.len() {
        return Err("Modrinth hat zu viele Projektlinks geliefert.".into());
    }
    let mut received = HashSet::new();
    let mut links = Vec::new();
    for project in details {
        if !requested.contains(&project.id) || !received.insert(project.id.clone()) {
            return Err("Modrinth hat unerwartete oder doppelte Projektlinks geliefert.".into());
        }
        links.push(ProjectLinks {
            project_id: project.id,
            source_url: project.source_url.as_deref().and_then(safe_source_url),
        });
    }
    Ok(links)
}

/// Optional search-result enrichment: one bounded bulk request, never one detail request per hit.
/// Repository links are grouping hints only; provider/project identities remain separate.
#[tauri::command]
pub async fn get_modrinth_project_links(
    project_ids: Vec<String>,
) -> Result<Vec<ProjectLinks>, String> {
    let requested = project_link_ids(project_ids)?;
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let mut response = client()?
        .get(format!("{API_BASE}/projects"))
        .query(&[("ids", json!(&requested).to_string())])
        .send()
        .await
        .map_err(|_| "Die zusätzlichen Modrinth-Projektlinks konnten nicht geladen werden.")?;
    if !response.status().is_success() {
        return Err(format!(
            "Modrinth-Projektlinks sind gerade nicht verfügbar (HTTP {}).",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_PROJECT_LINK_BYTES as u64)
    {
        return Err("Die Antwort mit Modrinth-Projektlinks ist zu groß.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Das Laden der Modrinth-Projektlinks wurde unterbrochen.")?
    {
        if bytes.len() + chunk.len() > MAX_PROJECT_LINK_BYTES {
            return Err("Die Antwort mit Modrinth-Projektlinks ist zu groß.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let details: Vec<ProjectDetails> = serde_json::from_slice(&bytes)
        .map_err(|_| "Modrinth hat ungültige Projektlinks geliefert.")?;
    project_links(details, &requested)
}

fn client() -> Result<&'static Client, String> {
    HTTP_CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
                .https_only(true)
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Der Modrinth-Client konnte nicht gestartet werden.".to_owned())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn bounded(value: &str, maximum: usize, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.chars().count() > maximum {
        return Err(format!("{label} ist zu lang."));
    }
    Ok(trimmed.to_owned())
}

fn selected(value: &str) -> bool {
    !value.is_empty() && value != "all"
}

async fn response_json(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "Modrinth antwortet gerade nicht. Bitte erneut versuchen.".to_owned()
        } else {
            "Modrinth ist gerade nicht erreichbar. Bitte die Verbindung prüfen.".to_owned()
        }
    })?;

    match response.status() {
        StatusCode::TOO_MANY_REQUESTS => {
            return Err("Zu viele Modrinth-Anfragen. Bitte kurz warten.".to_owned());
        }
        StatusCode::NOT_FOUND => {
            return Err("Dieses Modrinth-Projekt ist nicht mehr verfügbar.".to_owned());
        }
        status if !status.is_success() => {
            return Err(format!(
                "Modrinth konnte die Anfrage nicht beantworten ({status})."
            ));
        }
        _ => {}
    }

    response
        .json()
        .await
        .map_err(|_| "Die Antwort von Modrinth konnte nicht gelesen werden.".to_owned())
}

/// Returns the original search response, including hits and pagination metadata.
#[tauri::command]
pub async fn search_modrinth(
    query: String,
    game_version: String,
    loader: String,
    project_type: String,
) -> Result<Value, String> {
    let query = bounded(&query, 200, "Die Suchanfrage")?;
    let game_version = bounded(&game_version, 64, "Die Minecraft-Version")?;
    let loader = bounded(&loader, 64, "Der Modloader")?;
    let project_type = bounded(&project_type, 32, "Der Inhaltstyp")?;

    let mut facets = Vec::new();
    match project_type.as_str() {
        "datapack" => facets.push(vec!["all_project_types:datapack".to_owned()]),
        "mod" | "shader" | "resourcepack" | "modpack" => {
            facets.push(vec![format!("project_type:{project_type}")]);
        }
        _ => return Err("Dieser Modrinth-Inhaltstyp wird nicht unterstützt.".to_owned()),
    }
    if selected(&game_version) {
        facets.push(vec![format!("versions:{game_version}")]);
    }
    if project_type == "mod" && selected(&loader) {
        facets.push(vec![format!("categories:{loader}")]);
    }

    response_json(client()?.get(format!("{API_BASE}/search")).query(&[
        ("query", query),
        ("facets", json!(facets).to_string()),
        ("limit", "24".to_owned()),
        ("index", "relevance".to_owned()),
    ]))
    .await
}

/// Resolves versions without guessing compatibility from a project's display name.
/// projectType is optional for callers that only need the Minecraft-version filter.
#[tauri::command]
pub async fn get_modrinth_versions(
    project_id: String,
    game_version: String,
    loader: String,
    project_type: Option<String>,
) -> Result<Value, String> {
    let project_id = bounded(&project_id, 128, "Die Projekt-ID")?;
    if project_id.is_empty()
        || !project_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Die Modrinth-Projekt-ID ist ungültig.".to_owned());
    }
    let game_version = bounded(&game_version, 64, "Die Minecraft-Version")?;
    let loader = bounded(&loader, 64, "Der Modloader")?;

    let mut parameters = vec![("include_changelog", "false".to_owned())];
    if selected(&game_version) {
        parameters.push(("game_versions", json!([game_version]).to_string()));
    }
    // Shaders, resource packs and data packs are not Fabric/Forge mod artifacts.
    if project_type.as_deref() == Some("mod") && selected(&loader) {
        parameters.push(("loaders", json!([loader]).to_string()));
    }

    response_json(
        client()?
            .get(format!("{API_BASE}/project/{project_id}/version"))
            .query(&parameters),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_project_link_ids_are_bounded_and_never_become_url_paths() {
        assert!(project_link_ids(Vec::new()).unwrap().is_empty());
        assert_eq!(
            project_link_ids(vec!["AANobbMI".into(), "AANobbMI".into()])
                .unwrap()
                .len(),
            1
        );
        let maximum = (0..24).map(|index| format!("project{index}")).collect();
        assert_eq!(project_link_ids(maximum).unwrap().len(), 24);
        assert!(
            project_link_ids((0..25).map(|index| format!("project{index}")).collect()).is_err()
        );
        for id in [
            "",
            "../secret",
            "a/b",
            "a?ids=other",
            " a",
            "a\n",
            "https://example.com",
        ] {
            assert!(project_link_ids(vec![id.into()]).is_err(), "{id}");
        }
    }

    #[test]
    fn bulk_project_links_only_return_requested_unique_ids_and_safe_optional_source_urls() {
        let requested = project_link_ids(vec!["AANobbMI".into(), "PtjYWJkn".into()]).unwrap();
        let details:Vec<ProjectDetails>=serde_json::from_value(json!([
            {"id":"AANobbMI","source_url":"https://github.com/CaffeineMC/sodium?token=synthetic-secret#readme","unrelated_private_field":"synthetic-secret"},
            {"id":"PtjYWJkn","source_url":null}])).unwrap();
        let links = project_links(details, &requested).unwrap();
        assert_eq!(
            links[0].source_url.as_deref(),
            Some("https://github.com/CaffeineMC/sodium")
        );
        assert!(links[1].source_url.is_none());
        assert!(!serde_json::to_string(&links)
            .unwrap()
            .contains("synthetic-secret"));
        let unknown = vec![ProjectDetails {
            id: "unknown".into(),
            source_url: None,
        }];
        assert!(project_links(unknown, &requested).is_err());
        let repeated = vec![
            ProjectDetails {
                id: "AANobbMI".into(),
                source_url: None,
            },
            ProjectDetails {
                id: "AANobbMI".into(),
                source_url: None,
            },
        ];
        assert!(project_links(repeated, &requested).is_err());
    }

    #[test]
    fn repository_hints_reject_credentials_non_https_ports_and_control_characters() {
        for url in [
            "http://github.com/owner/repo",
            "https://name:secret@github.com/owner/repo",
            "https://github.com:444/owner/repo",
            "file:///etc/passwd",
            "https://github.com/owner/repo\n",
        ] {
            assert!(safe_source_url(url).is_none(), "{url}");
        }
        assert_eq!(
            safe_source_url("https://github.com/Owner/Repo.git").as_deref(),
            Some("https://github.com/Owner/Repo.git")
        );
    }
}
