use crate::{
    model::{Manifest, ToolVersion},
    CoreError, CoreResult,
};
use serde_json::Value;

/// A package can identify its build with a top-level version, while the Registry
/// describes downloadable versions with `versions`. Keep these contracts separate.
pub(crate) struct PackageManifest {
    manifest: Manifest,
    version: Option<String>,
    channel: Option<String>,
}

impl PackageManifest {
    pub(crate) fn from_value(raw: Value) -> CoreResult<Self> {
        let version: Option<String> = raw
            .get("version")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?;
        let channel: Option<String> = raw
            .get("channel")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?;
        let has_versions = raw.get("versions").is_some();
        let mut manifest: Manifest = serde_json::from_value(raw)?;

        if let Some(version) = &version {
            semver::Version::parse(version).map_err(|_| {
                CoreError::invalid(format!(
                    "tool.json 顶层 version={version:?} 必须为 MAJOR.MINOR.PATCH"
                ))
            })?;
            let channel = channel.as_deref().unwrap_or("stable");
            if !has_versions {
                manifest.versions.push(ToolVersion {
                    version: version.clone(),
                    channel: channel.into(),
                    package_url: String::new(),
                    sha256: String::new(),
                    release_date: String::new(),
                    size: None,
                    changelog: vec![],
                    min_toolbox_version: None,
                });
                if manifest.latest.is_empty() {
                    manifest.latest.insert(channel.into(), version.clone());
                }
            }
        } else if channel.is_some() {
            return Err(CoreError::invalid(
                "tool.json 顶层 channel 必须与顶层 version 一起声明",
            ));
        }

        manifest.validate()?;
        let package = Self {
            manifest,
            version,
            channel,
        };
        if let Some(version) = &package.version {
            let channel = package.channel.as_deref().unwrap_or("stable");
            if !package
                .manifest
                .versions
                .iter()
                .any(|item| &item.version == version && item.channel == channel)
            {
                return Err(CoreError::invalid(format!(
                    "tool.json 顶层 version={version:?}, channel={channel:?} 与 versions 声明={:?} 不一致",
                    package.declared_versions()
                )));
            }
            if let Some(latest) = package.manifest.latest.get(channel) {
                if latest != version {
                    return Err(CoreError::invalid(format!(
                        "tool.json 顶层 version={version:?}, channel={channel:?} 与 latest.{channel}={latest:?} 不一致"
                    )));
                }
            }
        }
        Ok(package)
    }

    pub(crate) fn validate_request(
        &self,
        registry: &Manifest,
        selected: &ToolVersion,
    ) -> CoreResult<()> {
        if self.manifest.id != registry.id {
            return Err(CoreError::invalid(format!(
                "包内 ID 与 Registry 不一致：Registry.id={:?}，tool.json.id={:?}",
                registry.id, self.manifest.id
            )));
        }
        if self.manifest.entry != registry.entry {
            return Err(CoreError::invalid(format!(
                "包内入口与 Registry 不一致：Registry.entry={:?}，tool.json.entry={:?}",
                registry.entry, self.manifest.entry
            )));
        }
        if self.manifest.versions.is_empty() {
            return Err(CoreError::invalid(
                "tool.json 缺少顶层 version 或非空 versions 声明",
            ));
        }
        let matches = if let Some(version) = &self.version {
            version == &selected.version
                && self.channel.as_deref().unwrap_or("stable") == selected.channel
        } else {
            self.manifest
                .versions
                .iter()
                .any(|item| item.version == selected.version && item.channel == selected.channel)
        };
        if !matches {
            return Err(CoreError::invalid(format!(
                "包内版本/通道与请求不一致：tool.json 声明={:?}",
                self.declared_versions()
            )));
        }
        Ok(())
    }

    fn declared_versions(&self) -> Vec<(&str, &str)> {
        self.manifest
            .versions
            .iter()
            .map(|item| (item.version.as_str(), item.channel.as_str()))
            .collect()
    }

    pub(crate) fn installed_version(&self) -> Option<&ToolVersion> {
        if let Some(version) = &self.version {
            let channel = self.channel.as_deref().unwrap_or("stable");
            self.manifest
                .versions
                .iter()
                .find(|item| &item.version == version && item.channel == channel)
        } else {
            self.manifest.latest.get("stable").and_then(|version| {
                self.manifest
                    .versions
                    .iter()
                    .find(|item| &item.version == version && item.channel == "stable")
            })
        }
    }

    pub(crate) fn into_manifest(self) -> Manifest {
        self.manifest
    }
}
