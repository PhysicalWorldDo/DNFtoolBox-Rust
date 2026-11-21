use semver::Version;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use toolbox_core::CoreError;

pub const PROJECT_URL: &str = "https://github.com/PhysicalWorldDo/DNFtoolBox-Rust";
const RELEASE_API: &str =
    "https://api.github.com/repos/PhysicalWorldDo/DNFtoolBox-Rust/releases/latest";

#[derive(Debug, Clone)]
pub struct Package {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct Release {
    version: String,
    page: String,
    notes: Vec<String>,
    package: Option<Package>,
}

#[derive(Default)]
pub enum ReleaseStatus {
    #[default]
    Unchecked,
    Checked(Option<Release>),
}

impl ReleaseStatus {
    pub fn view(&self, version: &str) -> Value {
        let release = match self {
            Self::Checked(release) => release.as_ref(),
            Self::Unchecked => None,
        };
        let has_update = release.is_some_and(|latest| newer(&latest.version, version));
        let package = release.and_then(|latest| latest.package.as_ref());
        json!({
            "version": version,
            "checked": matches!(self, Self::Checked(_)),
            "latestVersion": release.map(|latest| &latest.version),
            "releaseUrl": self.page(),
            "sizeBytes": package.map(|asset| asset.size),
            "changelog": release.map(|latest| &latest.notes),
            "hasUpdate": has_update,
            "canDownload": has_update && package.is_some(),
        })
    }

    pub fn package(&self, requested: &str, current: &str) -> Option<Package> {
        match self {
            Self::Checked(Some(release))
                if release.version == requested && newer(requested, current) =>
            {
                release.package.clone()
            }
            _ => None,
        }
    }

    pub fn page(&self) -> String {
        match self {
            Self::Checked(Some(release)) => release.page.clone(),
            _ => format!("{PROJECT_URL}/releases"),
        }
    }
}

pub fn newer(version: &str, current: &str) -> bool {
    let version = version.strip_prefix('v').unwrap_or(version);
    match (Version::parse(version), Version::parse(current)) {
        (Ok(version), Ok(current)) => {
            version.pre.is_empty() && version.build.is_empty() && version > current
        }
        _ => false,
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    state: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

impl GitHubRelease {
    fn into_release(self) -> Option<Release> {
        let version = self.tag_name.strip_prefix('v')?;
        let parsed = Version::parse(version).ok()?;
        if self.draft || self.prerelease || !parsed.pre.is_empty() || !parsed.build.is_empty() {
            return None;
        }
        let filename = format!("PhysicalWorldToolbox-{version}-win-x64.zip");
        let url = format!(
            "{PROJECT_URL}/releases/download/{}/{filename}",
            self.tag_name
        );
        let package = self.assets.into_iter().find_map(|asset| {
            if asset.name != filename
                || asset.state != "uploaded"
                || asset.size == 0
                || asset.browser_download_url != url
            {
                return None;
            }
            let sha = asset.digest.as_deref()?.strip_prefix("sha256:")?;
            if sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            Some(Package {
                url: asset.browser_download_url,
                sha256: sha.to_ascii_lowercase(),
                size: asset.size,
            })
        });
        let notes = self
            .body
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(|line| line.trim().trim_start_matches("- "))
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        Some(Release {
            version: version.into(),
            page: format!("{PROJECT_URL}/releases/tag/{}", self.tag_name),
            notes,
            package,
        })
    }
}

pub async fn check() -> Result<ReleaseStatus, CoreError> {
    let error = |message: &dyn std::fmt::Display| {
        CoreError::new("NETWORK", format!("无法查询 GitHub Releases。{message}"))
    };
    let response = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("PhysicalWorldToolbox/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| error(&e))?
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| error(&e))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(ReleaseStatus::Checked(None));
    }
    let body = response
        .error_for_status()
        .map_err(|e| error(&e))?
        .bytes()
        .await
        .map_err(|e| error(&e))?;
    let release: GitHubRelease = serde_json::from_slice(&body)
        .map_err(|e| CoreError::new("INVALID_RELEASE", format!("GitHub 发布信息格式无效。{e}")))?;
    Ok(ReleaseStatus::Checked(release.into_release()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_view(raw_release: Value, version: &str) -> Value {
        let release: GitHubRelease = serde_json::from_value(raw_release).unwrap();
        ReleaseStatus::Checked(release.into_release()).view(version)
    }

    fn github_release() -> Value {
        json!({
            "tag_name": "v0.4.10",
            "draft": false,
            "prerelease": false,
            "body": "- 初次发布",
            "assets": [{
                "name": "PhysicalWorldToolbox-0.4.10-win-x64.zip",
                "state": "uploaded",
                "size": 6400000,
                "digest": format!("sha256:{}", "a".repeat(64)),
                "browser_download_url": "https://github.com/PhysicalWorldDo/DNFtoolBox-Rust/releases/download/v0.4.10/PhysicalWorldToolbox-0.4.10-win-x64.zip"
            }]
        })
    }

    #[test]
    fn selects_the_newer_stable_portable_release_with_githubs_digest() {
        let view = release_view(github_release(), "0.4.9");
        assert_eq!(view["latestVersion"], "0.4.10");
        assert_eq!(view["hasUpdate"], true);
        assert_eq!(view["canDownload"], true);
        assert_eq!(view["sizeBytes"], 6400000);
    }

    #[test]
    fn a_missing_digest_only_offers_the_release_page() {
        let mut release = github_release();
        release["assets"][0]["digest"] = Value::Null;
        let view = release_view(release, "0.4.9");
        assert_eq!(view["hasUpdate"], true);
        assert_eq!(view["canDownload"], false);
        assert_eq!(
            view["releaseUrl"],
            "https://github.com/PhysicalWorldDo/DNFtoolBox-Rust/releases/tag/v0.4.10"
        );
    }

    #[test]
    fn ignores_drafts_prereleases_and_nondeterministic_tag_names() {
        for change in [
            json!({"draft": true}),
            json!({"prerelease": true}),
            json!({"tag_name": "v0.4.10-beta.1"}),
            json!({"tag_name": "v0.4.10+local"}),
        ] {
            let mut release = github_release();
            release
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert_eq!(release_view(release, "0.4.9")["hasUpdate"], false);
        }
    }

    #[test]
    fn refuses_to_download_equal_or_older_versions() {
        for version in ["0.4.10", "0.5.0"] {
            let view = release_view(github_release(), version);
            assert_eq!(view["hasUpdate"], false);
            assert_eq!(view["canDownload"], false);
        }
    }

    #[test]
    fn requires_the_portable_asset_from_the_expected_repository() {
        for change in [
            json!({"name": "source.zip"}),
            json!({"browser_download_url": "https://example.invalid/portable.zip"}),
            json!({"state": "starter"}),
            json!({"size": 0}),
        ] {
            let mut release = github_release();
            release["assets"][0]
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert_eq!(release_view(release, "0.4.9")["canDownload"], false);
        }
    }
}
