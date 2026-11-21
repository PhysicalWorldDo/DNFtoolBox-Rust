use crate::{
    catalog::validate_index,
    model::{now, Index, Manifest, ToolVersion},
    paths::{atomic_json, hash_file, managed, read_json, replace_file, valid_hash},
    settings::Settings,
    CoreError, CoreResult,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

pub(crate) fn check_cancel(cancel: &AtomicBool) -> CoreResult<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(CoreError::cancelled())
    } else {
        Ok(())
    }
}

fn client(timeout: u64) -> CoreResult<Client> {
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(timeout))
        .user_agent(concat!("PhysicalWorldToolbox/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| CoreError::new("NETWORK", e.to_string()))
}
pub(crate) fn resolve_url(base: &str, reference: &str) -> CoreResult<String> {
    if Path::new(reference).is_absolute() {
        return Ok(reference.into());
    }
    if let Ok(url) = url::Url::parse(reference) {
        if ["https", "http", "file"].contains(&url.scheme()) {
            return Ok(url.to_string());
        }
    }
    if let Ok(base) = url::Url::parse(base) {
        if ["https", "http", "file"].contains(&base.scheme()) {
            return base
                .join(reference)
                .map(|u| u.to_string())
                .map_err(|e| CoreError::invalid(e.to_string()));
        }
    }
    Ok(Path::new(base)
        .parent()
        .unwrap_or(Path::new("."))
        .join(reference)
        .to_string_lossy()
        .into_owned())
}
pub(crate) fn local_path(address: &str) -> Option<PathBuf> {
    if Path::new(address).is_absolute() {
        return Some(PathBuf::from(address));
    }
    if let Ok(parsed) = url::Url::parse(address) {
        if parsed.scheme() == "file" {
            return parsed.to_file_path().ok();
        }
        return None;
    }
    Some(PathBuf::from(address))
}
fn candidates(address: &str, settings: &Settings) -> Vec<String> {
    let github = url::Url::parse(address).ok().is_some_and(|u| {
        [
            "github.com",
            "raw.githubusercontent.com",
            "objects.githubusercontent.com",
            "api.github.com",
        ]
        .contains(&u.host_str().unwrap_or(""))
    });
    if !github || settings.raw["githubProxy"]["enabled"] != true {
        return vec![address.into()];
    }
    let encoded: String = url::form_urlencoded::byte_serialize(address.as_bytes()).collect();
    let mut urls: Vec<String> = settings.raw["githubProxy"]["urls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|proxy| {
            if proxy.contains("{encoded_url}") {
                proxy.replace("{encoded_url}", &encoded)
            } else if proxy.contains("{url}") {
                proxy.replace("{url}", address)
            } else {
                format!("{proxy}{address}")
            }
        })
        .collect();
    urls.push(address.into());
    urls
}
type Rankings = BTreeMap<String, (Instant, Vec<usize>)>;
static RANKINGS: OnceLock<Mutex<Rankings>> = OnceLock::new();
fn ranked_candidates(
    address: &str,
    settings: &Settings,
    cancel: &AtomicBool,
) -> CoreResult<Vec<String>> {
    let urls = candidates(address, settings);
    if urls.len() < 2 {
        return Ok(urls);
    }
    let key = format!(
        "{}|{}",
        url::Url::parse(address)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default(),
        settings.raw["githubProxy"]
    );
    let cache = RANKINGS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some((at, order)) = cache.lock().unwrap().get(&key) {
        if at.elapsed() < Duration::from_secs(60) {
            return Ok(order.iter().filter_map(|i| urls.get(*i).cloned()).collect());
        }
    }
    let cursor = AtomicUsize::new(0);
    let results = Mutex::new(vec![]);
    let probe = client(3)?;
    std::thread::scope(|scope| {
        for _ in 0..urls.len().min(8) {
            let urls = &urls;
            let probe = &probe;
            let cursor = &cursor;
            let results = &results;
            scope.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= urls.len() {
                    break;
                }
                let started = Instant::now();
                let elapsed = probe
                    .get(&urls[index])
                    .header("Range", "bytes=0-0")
                    .send()
                    .and_then(|r| r.error_for_status())
                    .ok()
                    .map(|_| started.elapsed());
                results.lock().unwrap().push((index, elapsed));
            });
        }
    });
    check_cancel(cancel)?;
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(index, time)| (time.is_none(), time.unwrap_or(Duration::MAX), *index));
    let order: Vec<usize> = results.iter().map(|(i, _)| *i).collect();
    cache
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), order.clone()));
    Ok(order.iter().map(|i| urls[*i].clone()).collect())
}
fn fetch_json(address: &str, settings: &Settings, cancel: &AtomicBool) -> CoreResult<Value> {
    check_cancel(cancel)?;
    if let Some(path) = local_path(address) {
        return read_json(&path);
    }
    let client = client(15)?;
    let mut last = CoreError::new("NETWORK", "没有可用下载节点");
    for candidate in ranked_candidates(address, settings, cancel)? {
        check_cancel(cancel)?;
        let result = (|| -> CoreResult<Value> {
            let mut response = client
                .get(candidate)
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| CoreError::new("NETWORK", e.to_string()))?;
            // Registry metadata is small; a bounded reader rejects accidental HTML/binary responses.
            let mut bytes = vec![];
            response
                .by_ref()
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(CoreError::invalid("Registry 清单过大"));
            }
            Ok(serde_json::from_slice(&bytes)?)
        })();
        match result {
            Ok(value) => return Ok(value),
            Err(error) => last = error,
        }
    }
    Err(last)
}
pub(crate) fn refresh(
    workspace: &Path,
    settings: &Settings,
    cancel: &AtomicBool,
) -> CoreResult<(Index, BTreeMap<String, Manifest>, String, String)> {
    let mut index: Index =
        serde_json::from_value(fetch_json(settings.index_url(), settings, cancel)?)?;
    validate_index(&index)?;
    if !index.toolbox.release_url.is_empty() {
        index.toolbox.release_url = resolve_url(settings.index_url(), &index.toolbox.release_url)?;
    }
    if !index.toolbox.package_url.is_empty() {
        index.toolbox.package_url = resolve_url(settings.index_url(), &index.toolbox.package_url)?;
    }
    let cursor = AtomicUsize::new(0);
    let results = Mutex::new(Vec::<CoreResult<Manifest>>::new());
    std::thread::scope(|scope| {
        for _ in 0..index.tools.len().min(8) {
            let cursor = &cursor;
            let results = &results;
            let tools = &index.tools;
            scope.spawn(move || loop {
                let item = cursor.fetch_add(1, Ordering::Relaxed);
                if item >= tools.len() {
                    break;
                }
                let result = (|| -> CoreResult<Manifest> {
                    check_cancel(cancel)?;
                    let tool = &tools[item];
                    let address = resolve_url(settings.index_url(), &tool.manifest_url)?;
                    let mut manifest: Manifest =
                        serde_json::from_value(fetch_json(&address, settings, cancel)?)?;
                    manifest.validate()?;
                    if manifest.id != tool.id {
                        return Err(CoreError::invalid(format!(
                            "索引与清单 ID 不一致：{}",
                            tool.id
                        )));
                    }
                    if manifest.versions.is_empty() {
                        return Err(CoreError::invalid("远端工具清单没有版本"));
                    }
                    for version in &mut manifest.versions {
                        // Legacy Registry histories may retain release notes after a package
                        // was withdrawn. Eligibility is checked for the selected install target.
                        if !version.package_url.is_empty() {
                            version.package_url = resolve_url(&address, &version.package_url)?;
                        }
                    }
                    if !manifest.icon.is_empty() {
                        manifest.icon = resolve_url(&address, &manifest.icon)?;
                    }
                    Ok(manifest)
                })();
                results.lock().unwrap().push(result);
            });
        }
    });
    let mut manifests = BTreeMap::new();
    for result in results.into_inner().unwrap() {
        let manifest = result?;
        manifests.insert(manifest.id.clone(), manifest);
    }
    check_cancel(cancel)?;
    let checked = now();
    let root = managed(workspace, "config/registry-cache")?;
    let generation = format!("g-{}", uuid::Uuid::new_v4().simple());
    let generation_root = managed(&root, &format!("generations/{generation}"))?;
    for (id, manifest) in &manifests {
        atomic_json(
            &managed(&generation_root, &format!("tools/{id}.json"))?,
            manifest,
        )?;
    }
    let mut cached_index = index.clone();
    for tool in &mut cached_index.tools {
        tool.manifest_url = format!("tools/{}.json", tool.id);
    }
    let mut value = serde_json::to_value(cached_index)?;
    value["checkedAt"] = json!(checked);
    atomic_json(&managed(&generation_root, "index.json")?, &value)?;
    check_cancel(cancel)?;
    Ok((index, manifests, checked, generation))
}
pub(crate) fn publish_generation(workspace: &Path, generation: &str) -> CoreResult<()> {
    crate::paths::validate_id(generation)?;
    let root = managed(workspace, "config/registry-cache")?;
    let previous = read_json(&root.join("current.json"))
        .ok()
        .and_then(|p| p["generation"].as_str().map(str::to_string));
    atomic_json(
        &managed(&root, "current.json")?,
        &json!({"generation":generation,"previous":previous}),
    )?;
    Ok(())
}

pub(crate) fn download(
    workspace: &Path,
    version: &ToolVersion,
    settings: &Settings,
    operation_id: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> CoreResult<PathBuf> {
    if !valid_hash(&version.sha256) {
        return Err(CoreError::invalid("缺少有效 SHA256"));
    }
    let target = managed(
        workspace,
        &format!("downloads/{}.zip", version.sha256.to_ascii_lowercase()),
    )?;
    fs::create_dir_all(target.parent().unwrap())?;
    if target.is_file() && hash_file(&target)?.eq_ignore_ascii_case(&version.sha256) {
        let size = fs::metadata(&target)?.len();
        progress(size, Some(size));
        return Ok(target);
    }
    let candidates = if local_path(&version.package_url).is_some() {
        vec![version.package_url.clone()]
    } else {
        ranked_candidates(&version.package_url, settings, cancel)?
    };
    let client = client(120)?;
    let mut last = CoreError::new("NETWORK", "没有可用下载节点");
    for address in candidates {
        check_cancel(cancel)?;
        let partial = managed(workspace, &format!("downloads/{operation_id}.part"))?;
        let result = (|| -> CoreResult<()> {
            let (mut reader, total): (Box<dyn Read>, Option<u64>) =
                if let Some(path) = local_path(&address) {
                    let file = fs::File::open(path)?;
                    let size = file.metadata()?.len();
                    (Box::new(file), Some(size))
                } else {
                    let response = client
                        .get(&address)
                        .send()
                        .and_then(|r| r.error_for_status())
                        .map_err(|e| CoreError::new("NETWORK", e.to_string()))?;
                    let total = response.content_length();
                    (Box::new(response), total)
                };
            if let Some(total) = total {
                ensure_space(workspace, total.saturating_add(16 * 1024 * 1024))?;
            }
            let mut output = fs::File::create(&partial)?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0; 256 * 1024];
            let mut done = 0;
            let mut last_progress = Instant::now() - Duration::from_secs(1);
            loop {
                check_cancel(cancel)?;
                let count = reader.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                output.write_all(&buffer[..count])?;
                digest.update(&buffer[..count]);
                done += count as u64;
                if last_progress.elapsed() >= Duration::from_millis(100) {
                    progress(done, total);
                    last_progress = Instant::now();
                }
            }
            output.sync_all()?;
            if total.is_some_and(|t| t != done) {
                return Err(CoreError::new(
                    "NETWORK",
                    "下载长度与 Content-Length 不一致",
                ));
            }
            if hex::encode(digest.finalize()) != version.sha256.to_ascii_lowercase() {
                return Err(CoreError::new(
                    "HASH_MISMATCH",
                    "安装包 SHA256 校验失败，旧工具未改动",
                ));
            }
            check_cancel(cancel)?;
            replace_file(&partial, &target)?;
            progress(done, total);
            Ok(())
        })();
        if partial.exists() {
            let _ = fs::remove_file(&partial);
        }
        match result {
            Ok(()) => return Ok(target),
            Err(error) if error.code == "CANCELLED" => return Err(error),
            Err(error) => last = error,
        }
    }
    Err(last)
}
pub(crate) fn ensure_space(workspace: &Path, required: u64) -> CoreResult<()> {
    if fs2::available_space(workspace)? < required {
        return Err(CoreError::new(
            "INSUFFICIENT_SPACE",
            format!("工作区至少需要 {} MB 可用空间", required / 1024 / 1024),
        ));
    }
    Ok(())
}
