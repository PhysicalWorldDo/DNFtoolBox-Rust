use crate::{
    compare_versions,
    model::{Index, Installed, Manifest, ToolVersion},
    package::PackageManifest,
    paths::{managed, plain_tree, read_json, valid_hash, validate_id},
    settings::Settings,
    CoreError, CoreResult,
};
use serde_json::{json, Value};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(crate) struct Catalog {
    pub index: Index,
    pub manifests: BTreeMap<String, Manifest>,
    pub installed: BTreeMap<String, Installed>,
    pub installed_error: Option<String>,
    pub source: String,
    pub checked_at: Option<String>,
    pub revision: u64,
    pub error: Option<String>,
    pub recovery: BTreeSet<String>,
}
impl Manifest {
    pub fn validate(&self) -> CoreResult<()> {
        if self.schema_version != 1 {
            return Err(CoreError::new(
                "UNSUPPORTED_VERSION",
                "不支持此工具清单版本",
            ));
        }
        validate_id(&self.id)?;
        crate::validate_entry(&self.entry)?;
        if !self.icon_source.is_empty() {
            crate::validate_entry(&self.icon_source)?;
        }
        let mut versions = BTreeSet::new();
        for version in &self.versions {
            semver::Version::parse(&version.version)
                .map_err(|_| CoreError::invalid("版本必须为 MAJOR.MINOR.PATCH"))?;
            if !versions.insert((version.channel.clone(), version.version.clone())) {
                return Err(CoreError::invalid("工具清单包含重复版本"));
            }
            if let Some(min) = &version.min_toolbox_version {
                semver::Version::parse(min)
                    .map_err(|_| CoreError::invalid("minToolboxVersion 无效"))?;
            }
        }
        for (channel, latest) in &self.latest {
            if !self
                .versions
                .iter()
                .any(|v| &v.channel == channel && &v.version == latest)
            {
                return Err(CoreError::invalid(format!("latest.{channel} 指向未知版本")));
            }
        }
        Ok(())
    }
    pub fn selected(
        &self,
        channel: &str,
        version: Option<&str>,
        app_version: &str,
    ) -> CoreResult<ToolVersion> {
        let target = version
            .or_else(|| self.latest.get(channel).map(String::as_str))
            .ok_or_else(|| CoreError::new("INCOMPATIBLE", "该渠道没有可安装版本"))?;
        let selected = self
            .versions
            .iter()
            .find(|v| v.version == target && v.channel == channel)
            .ok_or_else(|| CoreError::invalid("找不到指定渠道的工具版本"))?;
        if self.blocked_versions.contains(&selected.version) {
            return Err(CoreError::new(
                "BLOCKED_VERSION",
                "此版本已禁止安装，请选择其他版本",
            ));
        }
        if !compatible(selected, app_version) {
            return Err(CoreError::new(
                "INCOMPATIBLE",
                format!(
                    "此版本需要工具箱 {} 或更新版本",
                    selected.min_toolbox_version.as_deref().unwrap_or("未知")
                ),
            ));
        }
        if selected.package_url.is_empty() || !valid_hash(&selected.sha256) {
            return Err(CoreError::new("INCOMPATIBLE", "此版本没有可校验的安装包"));
        }
        Ok(selected.clone())
    }
}
pub(crate) fn compatible(version: &ToolVersion, app_version: &str) -> bool {
    version
        .min_toolbox_version
        .as_deref()
        .is_none_or(|min| compare_versions(app_version, min) != Ordering::Less)
}
pub(crate) fn validate_index(index: &Index) -> CoreResult<()> {
    if index.schema_version != 1 {
        return Err(CoreError::new(
            "UNSUPPORTED_VERSION",
            "不支持此 Registry 索引版本",
        ));
    }
    let mut seen = BTreeSet::new();
    for tool in &index.tools {
        validate_id(&tool.id)?;
        if !seen.insert(&tool.id) {
            return Err(CoreError::invalid("索引包含重复工具 ID"));
        }
    }
    Ok(())
}
pub(crate) fn load_installed(workspace: &Path) -> CoreResult<(BTreeMap<String, Installed>, Value)> {
    let path = managed(workspace, "config/installed.json")?;
    let raw = if path.exists() {
        read_json(&path)?
    } else {
        json!({"tools":{}})
    };
    if raw.get("schemaVersion").is_some_and(|v| v != 1) {
        return Err(CoreError::new(
            "UNSUPPORTED_VERSION",
            "不支持此安装记录版本，原文件已保留",
        ));
    }
    let tools = raw
        .get("tools")
        .and_then(Value::as_object)
        .ok_or_else(|| CoreError::invalid("installed.json 缺少 tools 对象"))?;
    let mut installed = BTreeMap::new();
    for (id, value) in tools {
        validate_id(id)?;
        let record: Installed = serde_json::from_value(value.clone())?;
        if record.id != *id {
            return Err(CoreError::invalid("安装记录的工具 ID 不一致"));
        }
        crate::validate_entry(&record.entry)?;
        installed.insert(id.clone(), record);
    }
    Ok((installed, raw))
}
fn load_cache(
    root: &Path,
    complete: bool,
) -> CoreResult<(Index, BTreeMap<String, Manifest>, Option<String>)> {
    let raw = read_json(&managed(root, "index.json")?)?;
    let checked_at = raw
        .get("checkedAt")
        .and_then(Value::as_str)
        .map(str::to_string);
    let index: Index = serde_json::from_value(raw)?;
    validate_index(&index)?;
    let mut manifests = BTreeMap::new();
    for tool in &index.tools {
        let result = (|| -> CoreResult<Manifest> {
            if tool.manifest_url != format!("tools/{}.json", tool.id) {
                return Err(CoreError::invalid("缓存清单路径无效"));
            }
            let manifest: Manifest = serde_json::from_value(read_json(&managed(
                root,
                &format!("tools/{}.json", tool.id),
            )?)?)?;
            manifest.validate()?;
            if manifest.id != tool.id {
                return Err(CoreError::invalid("索引和清单 ID 不一致"));
            }
            Ok(manifest)
        })();
        match result {
            Ok(manifest) => {
                manifests.insert(tool.id.clone(), manifest);
            }
            Err(e) if complete => return Err(e),
            Err(_) => {}
        }
    }
    Ok((index, manifests, checked_at))
}
impl Catalog {
    pub fn load(workspace: &Path) -> Self {
        let mut catalog = Self {
            index: Index::default(),
            manifests: BTreeMap::new(),
            installed: BTreeMap::new(),
            installed_error: None,
            source: "empty".into(),
            checked_at: None,
            revision: 1,
            error: None,
            recovery: BTreeSet::new(),
        };
        let root = workspace.join("config/registry-cache");
        let mut roots = vec![];
        if let Ok(pointer) = read_json(&root.join("current.json")) {
            for key in ["generation", "previous"] {
                if let Some(generation) = pointer[key].as_str() {
                    if validate_id(generation).is_ok() {
                        roots.push((root.join("generations").join(generation), true));
                    }
                }
            }
        }
        roots.push((root, false));
        for (root, complete) in roots {
            match load_cache(&root, complete) {
                Ok((index, manifests, checked_at)) => {
                    catalog.index = index;
                    catalog.manifests = manifests;
                    catalog.checked_at = checked_at;
                    catalog.source = "cached".into();
                    break;
                }
                Err(e) if root.join("index.json").exists() => {
                    catalog.error = Some(format!("目录缓存读取失败：{}", e.message))
                }
                Err(_) => {}
            }
        }
        match load_installed(workspace) {
            Ok((installed, _)) => catalog.installed = installed,
            Err(e) => {
                catalog.installed_error = Some(format!(
                    "安装记录不可写：{}。请打开 config 目录修复 installed.json。",
                    e.message
                ));
                catalog.error = catalog.installed_error.clone();
            }
        }
        if let Ok(entries) = fs::read_dir(workspace.join("tools")) {
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().into_owned();
                if validate_id(&id).is_err()
                    || !entry.path().is_dir()
                    || plain_tree(&entry.path()).is_err()
                {
                    continue;
                }
                let Ok(package) = read_json(&entry.path().join("tool.json"))
                    .and_then(PackageManifest::from_value)
                else {
                    continue;
                };
                let local_version = package
                    .installed_version()
                    .map(|v| (v.version.clone(), v.channel.clone()));
                let manifest = package.into_manifest();
                if manifest.id != id {
                    continue;
                }
                if let Some((version, channel)) = local_version {
                    catalog
                        .installed
                        .entry(id.clone())
                        .or_insert_with(|| Installed {
                            id: id.clone(),
                            name: manifest.name.clone(),
                            version,
                            channel,
                            entry: manifest.entry.clone(),
                            installed_at: String::new(),
                            updated_at: String::new(),
                            extra: BTreeMap::new(),
                        });
                }
                catalog.manifests.entry(id).or_insert(manifest);
            }
        }
        if catalog.source == "empty"
            && (!catalog.installed.is_empty() || !catalog.manifests.is_empty())
        {
            catalog.source = "local".into();
        }
        catalog
    }
    pub fn toolbox(&self, app_version: &str) -> Value {
        let update = &self.index.toolbox;
        let has_update = !update.latest_version.is_empty()
            && compare_versions(&update.latest_version, app_version) == Ordering::Greater;
        json!({"version":app_version,"latestVersion":update.latest_version,"minSupportedVersion":update.min_supported_version,"releaseUrl":update.release_url,"packageUrl":update.package_url,"sha256":update.sha256,"sizeBytes":update.size,"changelog":update.changelog,"hasUpdate":has_update,"canDownload":has_update && !update.package_url.is_empty() && valid_hash(&update.sha256)})
    }
    pub fn view(
        &self,
        workspace: &Path,
        settings: &Settings,
        app_version: &str,
        busy: &BTreeSet<String>,
    ) -> Value {
        let mut ids: Vec<String> = self.index.tools.iter().map(|t| t.id.clone()).collect();
        for id in self.manifests.keys().chain(self.installed.keys()) {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        let tools: Vec<Value> = ids
            .iter()
            .map(|id| self.tool(id, workspace, settings, app_version, busy))
            .collect();
        let mut categories: Vec<String> = vec![];
        for tool in &tools {
            let category = tool["category"].as_str().unwrap_or("其他工具").to_string();
            if !categories.contains(&category) {
                categories.push(category);
            }
        }
        let updates = tools
            .iter()
            .filter(|tool| tool["hasUpdate"] == true)
            .count();
        let new_installs = tools
            .iter()
            .filter(|tool| tool["installed"] == false)
            .count();
        json!({"tools":tools,"categories":categories,"source":self.source,"checkedAt":self.checked_at,"revision":self.revision,"updateCount":updates,"newInstallCount":new_installs,"error":self.error})
    }
    pub fn tool(
        &self,
        id: &str,
        workspace: &Path,
        settings: &Settings,
        app_version: &str,
        busy: &BTreeSet<String>,
    ) -> Value {
        let manifest = self.manifests.get(id);
        let installed = self.installed.get(id);
        let indexed = self.index.tools.iter().find(|t| t.id == id);
        let name = manifest
            .map(|m| m.name.as_str())
            .or_else(|| installed.map(|t| t.name.as_str()))
            .or_else(|| indexed.map(|t| t.name.as_str()))
            .unwrap_or(id);
        let category = manifest
            .map(|m| m.category.as_str())
            .or_else(|| indexed.map(|t| t.category.as_str()))
            .unwrap_or("其他工具");
        let latest = manifest.and_then(|m| m.latest.get(settings.channel()));
        let eligible =
            manifest.and_then(|m| m.selected(settings.channel(), None, app_version).ok());
        let has_update = installed.zip(eligible.as_ref()).is_some_and(|(old, new)| {
            compare_versions(&new.version, &old.version) == Ordering::Greater
        });
        let entry_state = if self.recovery.contains(id) {
            "recoveryRequired"
        } else if busy.contains(id) {
            "busy"
        } else if let Some(record) = installed {
            match managed(&workspace.join("tools").join(id), &record.entry) {
                Ok(path) if path.is_file() => "ready",
                _ => "missing",
            }
        } else {
            "notInstalled"
        };
        let mut versions: Vec<Value> = manifest.map(|m| m.versions.iter().filter(|v| v.channel == settings.channel()).map(|v| {
            let blocked = m.blocked_versions.contains(&v.version);
            let reason = m.selected(settings.channel(), Some(&v.version), app_version).err().map(|e| e.message);
            json!({"version":v.version,"channel":v.channel,"date":v.release_date,"sizeBytes":v.size,"changelog":v.changelog,"compatible":compatible(v,app_version),"blocked":blocked,"canInstall":reason.is_none(),"disabledReason":reason})
        }).collect()).unwrap_or_default();
        versions.sort_by(|a, b| {
            compare_versions(
                b["version"].as_str().unwrap_or(""),
                a["version"].as_str().unwrap_or(""),
            )
        });
        let mut actions = vec![];
        if entry_state == "ready" {
            actions.push("launch");
        }
        let writable =
            self.installed_error.is_none() && !self.recovery.contains(id) && !busy.contains(id);
        if writable {
            if installed.is_none() && eligible.is_some() {
                actions.push("install");
            }
            if has_update {
                actions.push("update");
            }
            if installed.is_some() {
                if installed.zip(manifest).is_some_and(|(old, manifest)| {
                    manifest
                        .selected(settings.channel(), Some(&old.version), app_version)
                        .is_ok()
                }) {
                    actions.push("reinstall");
                }
                if versions.iter().any(|v| v["canInstall"] == true) {
                    actions.push("switchVersion");
                }
                actions.push("uninstall");
            }
        }
        if installed.is_some() && workspace.join("tools").join(id).is_dir() {
            actions.push("openFolder");
        }
        if manifest.is_some_and(|m| !m.project_url.is_empty()) {
            actions.push("copyProjectUrl");
        }
        let disabled_reason = match entry_state {
            "missing" => Some("已安装工具的入口不存在，请重新安装或打开目录检查".to_string()),
            "busy" => Some("此工具正在进行安装操作".to_string()),
            "recoveryRequired" => Some("此工具需要恢复，旧文件备份已保留".to_string()),
            _ => self.installed_error.clone().or_else(|| {
                if manifest.is_none() {
                    Some("工具清单不可用；有效的已装入口仍可启动".into())
                } else {
                    manifest.and_then(|m| {
                        m.selected(settings.channel(), None, app_version)
                            .err()
                            .map(|e| e.message)
                    })
                }
            }),
        };
        json!({"id":id,"name":name,"category":category,"description":manifest.map(|m|m.description.as_str()).unwrap_or(""),"iconUrl":null,"icon":manifest.map(|m|m.icon.as_str()).unwrap_or(""),"iconSource":manifest.map(|m|m.icon_source.as_str()).unwrap_or(""),"installedVersion":installed.map(|t|t.version.as_str()),"latestVersion":latest,"availableVersions":versions,"entryState":entry_state,"actions":actions,"disabledReason":disabled_reason,"projectUrl":manifest.map(|m|m.project_url.as_str()).unwrap_or(""),"needAdmin":manifest.is_some_and(|m|m.need_admin),"hasUpdate":has_update,"installed":installed.is_some(),"installDirectory":workspace.join("tools").join(id).to_string_lossy()})
    }
}
