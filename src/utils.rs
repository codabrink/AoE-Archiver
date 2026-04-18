use anyhow::{Result, bail};
use serde_json::Value;
use sevenz_rust2::ArchiveReader;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

pub fn extract_7z(archive: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    let mut files = HashMap::new();

    let mut cursor = Cursor::new(archive);
    let mut archive = ArchiveReader::new(&mut cursor, "".into())?;

    archive.for_each_entries(|entry, reader| {
        let mut content = vec![];
        let _ = reader.read_to_end(&mut content);
        files.insert(entry.name.clone(), content);
        Ok(true)
    })?;

    Ok(files)
}

pub fn extract_zip(data: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    let reader = Cursor::new(data);
    let mut archive = ZipArchive::new(reader)?;
    let mut map = HashMap::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        map.insert(file.name().to_string(), contents);
    }

    Ok(map)
}

pub fn extract_tar_gz(data: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let reader = Cursor::new(data);
    let decoder = GzDecoder::new(reader);
    let mut archive = Archive::new(decoder);
    let mut map = HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents)?;
        map.insert(path, contents);
    }

    Ok(map)
}

pub fn desktop_dir() -> Result<PathBuf> {
    let Some(desktop_dir) = dirs::desktop_dir() else {
        bail!("Missing desktop dir.");
    };
    Ok(desktop_dir)
}

pub fn validate_aoe2_source(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("Directory does not exist");
    }
    if !path.is_dir() {
        bail!("Path is not a directory");
    }

    // Check for AoE2DE executable
    let exe_path = path.join("AoE2DE_s.exe");
    if !exe_path.exists() {
        bail!("This doesn't appear to be an AoE2 DE directory (AoE2DE_s.exe not found)");
    }

    Ok(())
}

pub fn gh_download_url(
    gh_user: &str,
    gh_repo: &str,
    version: Option<&str>,
    search: &[&str],
) -> Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{gh_user}/{gh_repo}/releases");

    // Ask the api for the latest release download
    let client = reqwest::blocking::Client::new();
    let json = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:143.0) Gecko/20100101 Firefox/143.0",
        )
        .send()?
        .text()?;
    let json: Value = serde_json::from_str(&json)?;

    let Some(releases) = json.as_array() else {
        bail!("Expected releases json to be an array.");
    };
    if releases.is_empty() {
        bail!("{gh_repo} has no releases.");
    }

    let release = if let Some(version) = version {
        let Some(release) = releases.iter().find(|r| {
            r.get("tag_name")
                .and_then(|r| r.as_str())
                .is_some_and(|r| r.contains(version))
        }) else {
            return Ok(None);
        };
        release
    } else {
        releases.iter().nth(0).unwrap()
    };

    let Some(assets) = release.get("assets") else {
        bail!("Unexpected response from github: expected assets field.");
    };
    let Some(assets) = assets.as_array() else {
        bail!("Expected github assets to be an array, but it was not.");
    };

    for asset in assets {
        let Some(name) = asset.get("name").and_then(|n| n.as_str()) else {
            continue;
        };

        if !search.iter().all(|s| name.contains(s)) {
            continue;
        }

        let Some(url) = asset.get("browser_download_url").and_then(|u| u.as_str()) else {
            continue;
        };

        return Ok(Some(url.to_string()));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use crate::utils::gh_download_url;

    #[test]
    fn load_specific_version() {
        let result = gh_download_url(
            "luskaner",
            "ageLANServerLauncherCompanion",
            Some("v1.2.1.0"),
            &[],
        )
        .unwrap();

        assert_eq!(
            result.unwrap(),
            "https://github.com/luskaner/ageLANServerLauncherCompanion/releases/download/v1.2.1.0/ageLANServerLauncherCompanion_Age2FakeOnline_1.0.0.0.zip"
        );
    }
}
