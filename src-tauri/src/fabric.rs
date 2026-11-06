//! Fabric's official profile API supplies the main class and extra Maven libraries.
use crate::minecraft::{self, Download};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const META: &str = "https://meta.fabricmc.net/v2/versions/loader";

#[derive(Clone, Debug, Serialize)]
pub struct FabricLoader {
    pub version: String,
    pub stable: bool,
}

fn component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+- ".contains(&c))
    {
        return Err("Ungültige Fabric- oder Minecraft-Version.".into());
    }
    Ok(())
}

async fn json_url(url: reqwest::Url) -> Result<Value, String> {
    minecraft::client()?
        .get(url)
        .send()
        .await
        .map_err(|_| "Fabric ist nicht erreichbar.")?
        .error_for_status()
        .map_err(|e| {
            format!(
                "Fabric-Metadaten nicht verfügbar: {}",
                e.status().map(|s| s.as_u16()).unwrap_or(0)
            )
        })?
        .json()
        .await
        .map_err(|_| "Ungültige Fabric-Metadaten.".into())
}

fn endpoint(game: &str, loader: Option<&str>) -> Result<reqwest::Url, String> {
    component(game)?;
    let mut url = reqwest::Url::parse(META).unwrap();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "Ungültige Fabric-Adresse.")?;
        path.push(game);
        if let Some(loader) = loader {
            component(loader)?;
            path.push(loader).push("profile").push("json");
        }
    }
    Ok(url)
}

#[tauri::command]
pub async fn get_fabric_loaders(game_version: String) -> Result<Vec<FabricLoader>, String> {
    let list = json_url(endpoint(&game_version, None)?).await?;
    let list = list.as_array().ok_or("Ungültige Fabric-Loaderliste.")?;
    let mut loaders = Vec::new();
    for item in list {
        if let Some(version) = item["loader"]["version"].as_str() {
            component(version)?;
            loaders.push(FabricLoader {
                version: version.into(),
                stable: item["loader"]["stable"] == true,
            });
        }
    }
    if loaders.is_empty() {
        return Err("Fabric unterstützt diese Minecraft-Version noch nicht.".into());
    }
    Ok(loaders)
}

pub(crate) fn maven_path(coordinate: &str) -> Result<PathBuf, String> {
    let parts: Vec<&str> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || parts.iter().any(|p| {
            p.is_empty()
                || p.len() > 160
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
        })
    {
        return Err("Ungültige Maven-Koordinate in Fabric-Metadaten.".into());
    }
    let classifier = parts.get(3).map(|p| format!("-{p}")).unwrap_or_default();
    let value = format!(
        "{}/{}/{}/{}-{}{}.jar",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2],
        classifier
    );
    minecraft::relative_path(&value)
}

pub(crate) struct FabricInstall {
    pub version: Value,
    pub loader_version: String,
    pub files: usize,
    pub bytes: u64,
}

/// Normalize Fabric libraries to Mojang download descriptors, so launch verification is shared.
pub(crate) async fn install(
    root: &Path,
    game_version: &str,
    requested: Option<&str>,
    mut base: Value,
) -> Result<FabricInstall, String> {
    let loader_version = match requested.filter(|s| !s.is_empty()) {
        Some(version) => {
            component(version)?;
            version.to_owned()
        }
        None => {
            get_fabric_loaders(game_version.into())
                .await?
                .into_iter()
                .find(|l| l.stable)
                .ok_or("Keine stabile Fabric-Version verfügbar; bitte eine Version auswählen.")?
                .version
        }
    };
    let profile = json_url(endpoint(game_version, Some(&loader_version))?).await?;
    if profile["inheritsFrom"] != game_version
        || !profile["mainClass"]
            .as_str()
            .is_some_and(|s| s.starts_with("net.fabricmc.") && !s.contains(char::is_whitespace))
    {
        return Err("Das Fabric-Profil passt nicht zur ausgewählten Minecraft-Version.".into());
    }
    let libraries = profile["libraries"]
        .as_array()
        .ok_or("Fabric-Bibliotheken fehlen.")?;
    let mut normalized = Vec::new();
    let mut verified_bytes = 0;
    for library in libraries {
        let name = library["name"]
            .as_str()
            .ok_or("Fabric-Bibliotheksname fehlt.")?;
        let relative = maven_path(name)?;
        let repository_path = relative.to_string_lossy().replace('\\', "/");
        // Fabric mirrors every supported profile dependency on its own Maven server.
        let base_url = library["url"]
            .as_str()
            .unwrap_or("https://maven.fabricmc.net/");
        if base_url != "https://maven.fabricmc.net/" {
            return Err("Unbekannte Fabric-Bibliotheksquelle.".into());
        }
        let url = format!("{base_url}{repository_path}");
        let hash = if let Some(hash) = library["sha1"].as_str() {
            hash.to_owned()
        } else {
            let response = minecraft::client()?
                .get(format!("{url}.sha1"))
                .send()
                .await
                .map_err(|_| "Fabric-Prüfsumme konnte nicht geladen werden.")?
                .error_for_status()
                .map_err(|_| {
                    "Fabric veröffentlicht keine gültige Prüfsumme für diese Bibliothek."
                })?;
            if response
                .content_length()
                .is_some_and(|length| length > 1024)
            {
                return Err("Ungültige Fabric-Prüfsumme.".into());
            }
            let text = response
                .text()
                .await
                .map_err(|_| "Fabric-Prüfsumme konnte nicht gelesen werden.")?;
            if text.len() > 1024 {
                return Err("Ungültige Fabric-Prüfsumme.".into());
            }
            text.split_whitespace().next().unwrap_or("").to_owned()
        };
        if hash.len() != 40 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Ungültige Fabric-SHA-1-Prüfsumme.".into());
        }
        let path = PathBuf::from("libraries").join(&relative);
        let download = Download {
            url: url.clone(),
            sha1: hash.to_lowercase(),
            size: library["size"].as_u64(),
            path: path.clone(),
        };
        minecraft::download(root, &download).await?;
        let size = std::fs::metadata(minecraft::managed_path(root, &path)?)
            .map_err(|e| e.to_string())?
            .len();
        verified_bytes += size;
        normalized.push(json!({"name":name,"downloads":{"artifact":{"path":repository_path,"url":url,"sha1":hash.to_lowercase(),"size":size}}}));
    }
    let files = normalized.len();
    // Fabric entries precede the base; a matching group/artifact/classifier replaces the old one.
    let keys: std::collections::HashSet<String> = normalized
        .iter()
        .filter_map(|l| l["name"].as_str())
        .map(library_key)
        .collect();
    let original = base["libraries"]
        .as_array()
        .ok_or("Minecraft-Bibliotheken fehlen.")?;
    normalized.extend(
        original
            .iter()
            .filter(|l| {
                l["name"]
                    .as_str()
                    .is_none_or(|name| !keys.contains(&library_key(name)))
            })
            .cloned(),
    );
    base["libraries"] = Value::Array(normalized);
    base["mainClass"] = profile["mainClass"].clone();
    for kind in ["jvm", "game"] {
        if let Some(extra) = profile["arguments"][kind].as_array() {
            if base["arguments"][kind].is_null() {
                base["arguments"][kind] = json!([]);
            }
            base["arguments"][kind]
                .as_array_mut()
                .ok_or("Ungültige Minecraft-Startargumente.")?
                .extend(extra.iter().cloned());
        }
    }
    base["syncmcLoader"] = json!({"type":"fabric","version":loader_version});
    Ok(FabricInstall {
        version: base,
        loader_version,
        files,
        bytes: verified_bytes,
    })
}

fn library_key(name: &str) -> String {
    let parts: Vec<_> = name.split(':').collect();
    format!(
        "{}:{}:{}",
        parts.first().unwrap_or(&""),
        parts.get(1).unwrap_or(&""),
        parts.get(3).unwrap_or(&"")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maven_coordinates_cannot_escape_libraries() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.20.1").unwrap(),
            PathBuf::from("net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar")
        );
        for name in [
            "../evil:mod:1",
            "net.fabricmc:a:../../evil",
            "a:b:1:../../evil",
            "a:b:1@zip",
            "a::1",
        ] {
            assert!(maven_path(name).is_err(), "{name}");
        }
        assert_eq!(
            library_key("org.ow2.asm:asm:9.0"),
            library_key("org.ow2.asm:asm:9.10")
        );
    }
}
