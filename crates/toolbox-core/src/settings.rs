use crate::{
    paths::{atomic_json, read_json},
    CoreError, CoreResult,
};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const INDEX_URL: &str =
    "https://raw.githubusercontent.com/PhysicalWorldDo/DNFTOOLBOX-Registry/main/index.json";
pub(crate) const PROXIES: &[&str] = &[
    "https://ghfast.top/",
    "https://gh-proxy.com/",
    "https://ghproxy.net/",
    "https://github.akams.cn/",
    "https://gitproxy.click/",
    "https://gh.llkk.cc/",
    "https://github.tbedu.top/",
    "https://ghfile.geekertao.top/",
    "https://cdn.gh-proxy.com/",
    "https://github.dpik.top/",
    "https://gh.dpik.top/",
    "https://j.1lin.dpdns.org/",
    "https://github.starrlzy.cn/",
    "https://github-proxy.memory-echoes.cn/",
    "https://gh.felicity.ac.cn/",
    "https://cdn.akaere.online/",
    "https://slink.ltd/",
    "https://github.tmby.shop/",
    "https://ghpr.cc/",
    "https://gh.tryxd.cn/",
    "https://github.chenc.dev/",
    "https://gh.ddlc.top/",
    "https://gitproxy.mrhjx.cn/",
    "https://gh.sixyin.com/",
    "https://gh.monlor.com/",
];
#[derive(Clone)]
pub(crate) struct Settings {
    pub raw: Value,
    pub revision: u64,
}
impl Settings {
    pub fn load(workspace: &Path) -> CoreResult<Self> {
        let path = workspace.join("config/app.json");
        let mut raw = if path.exists() {
            read_json(&path)?
        } else {
            json!({})
        };
        if !raw.is_object() {
            return Err(CoreError::invalid("app.json 必须为对象"));
        }
        let defaults = json!({"name":"物理世界的工具箱","indexUrl":INDEX_URL,"channel":"stable","githubProxy":{"enabled":true,"urls":PROXIES},"suppressDirectoryWarning":false});
        for (key, value) in defaults.as_object().unwrap() {
            if raw.get(key).is_none() {
                raw[key] = value.clone();
            }
        }
        let old_index = raw["indexUrl"].as_str().unwrap_or(INDEX_URL).to_string();
        if old_index == "https://github.com/PhysicalWorldDo/DNFTOOLBOX-Registry/raw/refs/heads/main/index.json" || old_index.replace('\\', "/").ends_with("/examples/remote-index/index.json") && !Path::new(&old_index).exists() { raw["indexUrl"] = json!(INDEX_URL); }
        if !raw["githubProxy"].is_object() {
            raw["githubProxy"] = defaults["githubProxy"].clone();
        }
        if !raw["githubProxy"]["enabled"].is_boolean() {
            raw["githubProxy"]["enabled"] = json!(true);
        }
        let urls = normalize_urls(&raw["githubProxy"]["urls"])
            .unwrap_or_else(|_| PROXIES.iter().map(|s| s.to_string()).collect());
        let legacy: Vec<String> = PROXIES[..5].iter().map(|s| s.to_string()).collect();
        raw["githubProxy"]["urls"] = json!(if urls.is_empty() || urls == legacy {
            PROXIES.iter().map(|s| s.to_string()).collect()
        } else {
            urls
        });
        Ok(Self { raw, revision: 1 })
    }
    pub fn view(&self) -> Value {
        let mut view = self.raw.clone();
        view["revision"] = json!(self.revision);
        view
    }
    pub fn channel(&self) -> &str {
        self.raw["channel"].as_str().unwrap_or("stable")
    }
    pub fn index_url(&self) -> &str {
        self.raw["indexUrl"].as_str().unwrap_or(INDEX_URL)
    }
    pub fn update(&mut self, workspace: &Path, patch: &Value) -> CoreResult<()> {
        let object = patch
            .as_object()
            .ok_or_else(|| CoreError::invalid("patch 必须为对象"))?;
        let mut candidate = self.raw.clone();
        for (key, value) in object {
            match key.as_str() {
                "name" | "indexUrl" | "channel"
                    if value.as_str().is_some_and(|v| !v.trim().is_empty()) =>
                {
                    candidate[key] = value.clone();
                }
                "suppressDirectoryWarning" if value.is_boolean() => {
                    candidate[key] = value.clone();
                }
                "githubProxy" if value.is_object() => {
                    for (proxy_key, proxy_value) in value.as_object().unwrap() {
                        match proxy_key.as_str() {
                            "enabled" if proxy_value.is_boolean() => {
                                candidate[key][proxy_key] = proxy_value.clone()
                            }
                            "urls" => {
                                candidate[key][proxy_key] = json!(normalize_urls(proxy_value)?)
                            }
                            _ => return Err(CoreError::invalid("代理设置字段无效")),
                        }
                    }
                }
                _ => return Err(CoreError::invalid(format!("设置字段无效：{key}"))),
            }
        }
        atomic_json(&workspace.join("config/app.json"), &candidate)?;
        self.raw = candidate;
        self.revision += 1;
        Ok(())
    }
}
pub(crate) fn normalize_urls(value: &Value) -> CoreResult<Vec<String>> {
    let values = value
        .as_array()
        .ok_or_else(|| CoreError::invalid("代理 urls 必须为数组"))?;
    let mut normalized = vec![];
    for value in values {
        let value = value
            .as_str()
            .ok_or_else(|| CoreError::invalid("代理地址必须为字符串"))?
            .trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let mut address = if value.contains("://") {
            value.to_string()
        } else {
            format!("https://{value}")
        };
        let check = address
            .replace("{encoded_url}", "https%3A%2F%2Fexample.com")
            .replace("{url}", "https://example.com");
        let parsed = url::Url::parse(&check).map_err(|_| CoreError::invalid("代理地址格式错误"))?;
        if !["https", "http"].contains(&parsed.scheme())
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(CoreError::invalid("代理地址需要 http(s) 且不能含密码"));
        }
        if !address.contains('{') && !address.ends_with('/') {
            address.push('/');
        }
        if !normalized.contains(&address) {
            normalized.push(address);
        }
    }
    Ok(normalized)
}
