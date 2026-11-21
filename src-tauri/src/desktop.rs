use crate::icons::IconService;
use crate::{
    cli::{self, Arguments, Mode},
    instance::InstanceGuard,
    native,
    release::{self, Package, ReleaseStatus},
    self_update::{self, Stage, Transaction},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use toolbox_core::{CoreError, NativeAction, Request, Response, Service};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
struct DownloadTask {
    snapshot: Value,
    cancel: Arc<AtomicBool>,
}

pub struct HostState {
    pub service: Arc<Service>,
    downloads: Mutex<BTreeMap<String, DownloadTask>>,
    release: Mutex<ReleaseStatus>,
    completed_requests: Mutex<BTreeMap<String, (String, Response)>>,
    native_dispatch: tokio::sync::Mutex<()>,
    pub closing: AtomicBool,
    handoff_active: AtomicBool,
    pub exit_allowed: AtomicBool,
    pub ready: AtomicBool,
    trial: Option<String>,
    icons: Arc<IconService>,
    icon_network_enabled: bool,
}

impl HostState {
    fn new(service: Arc<Service>, trial: Option<String>, icon_network_enabled: bool) -> Arc<Self> {
        let icons = Arc::new(IconService::new(service.workspace()));
        let mut downloads = BTreeMap::new();
        if let Ok(entries) = fs::read_dir(service.workspace().join("cache/self-update")) {
            for entry in entries.flatten() {
                if let Ok(tx) =
                    Transaction::load(service.workspace(), &entry.file_name().to_string_lossy())
                {
                    if tx.stage == Stage::Prepared && release::newer(&tx.version, VERSION) {
                        downloads.insert(tx.id.clone(), DownloadTask { snapshot: json!({"id":tx.id,"kind":"toolboxUpdate","toolId":null,"version":tx.version,"state":"succeeded","stage":"complete","doneBytes":null,"totalBytes":null,"canCancel":false,"error":null,"result":{"transactionId":tx.id,"readyToApply":true},"children":[]}), cancel: Arc::new(AtomicBool::new(false)) });
                    }
                }
            }
        }
        Arc::new(Self {
            service,
            downloads: Mutex::new(downloads),
            release: Mutex::new(ReleaseStatus::default()),
            completed_requests: Mutex::new(BTreeMap::new()),
            native_dispatch: tokio::sync::Mutex::new(()),
            closing: AtomicBool::new(false),
            handoff_active: AtomicBool::new(false),
            exit_allowed: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            trial,
            icons,
            icon_network_enabled,
        })
    }
    pub fn busy(&self) -> bool {
        self.handoff_active.load(Ordering::Acquire)
            || self.service.is_busy()
            || self
                .downloads
                .lock()
                .unwrap()
                .values()
                .any(|op| matches!(op.snapshot["state"].as_str(), Some("queued" | "running")))
    }
    fn snapshot(&self) -> Value {
        let mut result = self.service.bootstrap();
        result["toolbox"] = self.toolbox();
        result["about"]["githubUrl"] = json!(release::PROJECT_URL);
        result["about"]["feedbackUrl"] = json!(format!("{}/issues", release::PROJECT_URL));
        self.enrich_catalog(&mut result["catalog"]);
        self.schedule_icons(&result["catalog"], &result["settings"]);
        if let Some(operations) = result["operations"].as_array_mut() {
            operations.extend(
                self.downloads
                    .lock()
                    .unwrap()
                    .values()
                    .map(|op| op.snapshot.clone()),
            );
        }
        result["host"] = json!({"version":VERSION,"webviewVersion":tauri::webview_version().ok(),"embeddedFrontend":cfg!(feature="custom-protocol"),"pendingUpdateTrial":self.trial_pending()});
        if self.trial_pending() {
            result["capabilities"]["install"] = json!(false);
            result["capabilities"]["selfUpdate"] = json!(false);
        }
        result
    }
    fn toolbox(&self) -> Value {
        self.release.lock().unwrap().view(VERSION)
    }
    async fn check_update(&self) -> Result<Value, CoreError> {
        let status = release::check().await?;
        let view = status.view(VERSION);
        let mut current = self.release.lock().unwrap();
        *current = status;
        self.service.emit("toolbox.changed", view.clone());
        Ok(view)
    }
    fn enrich_catalog(&self, catalog: &mut Value) {
        let revision = catalog["revision"].as_u64().unwrap_or(0);
        if let Some(tools) = catalog["tools"].as_array_mut() {
            for tool in tools {
                self.icons.enrich_tool(tool, revision);
            }
        }
    }
    fn schedule_icons(&self, catalog: &Value, settings: &Value) {
        if !self.icon_network_enabled
            || self.closing.load(Ordering::Relaxed)
            || self.trial_pending()
        {
            return;
        }
        for job in self.icons.remote_jobs(catalog, settings) {
            let service = self.service.clone();
            let icons = self.icons.clone();
            let candidates = download_candidates(&job.url, settings);
            tauri::async_runtime::spawn(async move {
                let Ok(client) = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(Duration::from_secs(12))
                    .user_agent(concat!("PhysicalWorldToolbox/", env!("CARGO_PKG_VERSION")))
                    .build()
                else {
                    return;
                };
                for url in candidates {
                    let result = async {
                        let mut response = client
                            .get(url)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?
                            .error_for_status()
                            .map_err(|e| e.to_string())?;
                        if response
                            .content_length()
                            .is_some_and(|length| length > 2 * 1024 * 1024)
                        {
                            return Err("工具图标超出大小限制。".to_owned());
                        }
                        let mut bytes = vec![];
                        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                                return Err("工具图标超出大小限制。".to_owned());
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        icons.store_remote(&job, &bytes)
                    }
                    .await;
                    if result.is_ok() {
                        service.emit("catalog.changed", service.bootstrap()["catalog"].clone());
                        return;
                    }
                }
                log(
                    service.workspace(),
                    "icon-unavailable",
                    json!({"toolId":job.id,"message":"图标读取失败，界面使用名称首字。"}),
                );
            });
        }
    }
    fn trial_pending(&self) -> bool {
        self.trial.as_ref().is_some_and(|id| {
            Transaction::load(self.service.workspace(), id)
                .map(|tx| tx.stage != Stage::Committed)
                .unwrap_or(true)
        })
    }
    fn cancel_all(&self) {
        self.service.cancel_cancellable();
        for task in self.downloads.lock().unwrap().values() {
            if task.snapshot["canCancel"] == true {
                task.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    fn change_download(&self, id: &str, patch: Value) {
        let value = {
            let mut tasks = self.downloads.lock().unwrap();
            let Some(task) = tasks.get_mut(id) else {
                return;
            };
            for (key, value) in patch.as_object().unwrap() {
                task.snapshot[key] = value.clone();
            }
            task.snapshot.clone()
        };
        self.service.emit("operation.changed", value);
    }
    async fn dispatch(
        self: &Arc<Self>,
        request: Request,
        window: Option<tauri::WebviewWindow>,
    ) -> Response {
        if request.v != 1 {
            return Response::failure(
                request.id,
                CoreError::new(
                    "UNSUPPORTED_VERSION",
                    "不支持此桥接协议版本，请重启工具箱。",
                ),
            );
        }
        if request.id.is_empty() || !request.params.is_object() {
            return Response::failure(
                request.id,
                CoreError::invalid("请求需要 id 和 params 对象。"),
            );
        }
        let method = request.method.as_str();
        let writes = matches!(
            method,
            "operation.start"
                | "operation.retry"
                | "settings.update"
                | "preferences.setPinned"
                | "preferences.selectTool"
                | "toolbox.downloadUpdate"
                | "toolbox.applyUpdate"
        );
        if writes
            && (self.closing.load(Ordering::Relaxed)
                || self.handoff_active.load(Ordering::Acquire)
                || self.trial_pending())
        {
            return Response::failure(
                request.id,
                CoreError::new("BUSY", "工具箱正在关闭或确认本体更新，请稍后重试。"),
            );
        }
        if method == "app.bootstrap" {
            return Response::success(request.id, self.snapshot());
        }
        if method == "toolbox.checkUpdate" {
            return match self.check_update().await {
                Ok(view) => Response::success(request.id, view),
                Err(error) => Response::failure(request.id, error),
            };
        }
        if method == "window.state" {
            // A state read must stay live and must not wait behind an update handoff.
            return match self.window_action(method, &request.params, window.as_ref()) {
                Ok(result) => Response::success(request.id, result),
                Err(error) => Response::failure(request.id, error),
            };
        }
        if method == "app.ready" {
            let response = self.service.dispatch(request).await;
            if response.ok
                && response
                    .result
                    .as_ref()
                    .is_some_and(|value| value["accepted"] == true)
                && window.is_some()
            {
                self.ready.store(true, Ordering::Relaxed);
                if let Some(id) = &self.trial {
                    if let Err(error) = self_update::acknowledge_ready(
                        self.service.workspace(),
                        id,
                        VERSION,
                        &self.service.session_id(),
                    ) {
                        log(
                            self.service.workspace(),
                            "trial-readiness-error",
                            json!({"message":error}),
                        );
                        return Response::failure(
                            response.id,
                            CoreError::new("RECOVERY_REQUIRED", error),
                        );
                    }
                }
                let proof = json!({"stage":"ready","version":VERSION,"pid":std::process::id(),"sessionId":self.service.session_id(),"workspace":self.service.workspace(),"embeddedFrontend":cfg!(feature="custom-protocol"),"webviewVersion":tauri::webview_version().ok(),"unixMs":unix_ms()});
                let directory = self.service.workspace().join("cache/logs");
                let _ = fs::create_dir_all(&directory);
                let _ = self_update::atomic_json(&directory.join("startup.json"), &proof);
                log(self.service.workspace(), "app.ready", proof);
            }
            return response;
        }
        if method == "operation.list" {
            return Response::success(request.id, self.snapshot()["operations"].clone());
        }
        if method == "operation.cancel" {
            let id = request.params["operationId"].as_str().unwrap_or_default();
            if let Some(task) = self.downloads.lock().unwrap().get(id) {
                if task.snapshot["canCancel"] == true {
                    task.cancel.store(true, Ordering::Relaxed);
                    return Response::success(request.id, json!({"accepted":true}));
                }
                return Response::failure(
                    request.id,
                    CoreError::new("BUSY", "任务已完成或正在准备安全交接。"),
                );
            }
        }
        let retry_version = if method == "operation.retry" {
            self.downloads
                .lock()
                .unwrap()
                .get(request.params["operationId"].as_str().unwrap_or_default())
                .map(|task| task.snapshot["version"].clone())
        } else {
            None
        };
        let host_method = retry_version.is_some()
            || matches!(
                method,
                "toolbox.downloadUpdate"
                    | "toolbox.applyUpdate"
                    | "tool.openFolder"
                    | "tool.copyProjectUrl"
                    | "system.openLink"
                    | "system.openWorkspace"
                    | "system.openLog"
                    | "about.copyContact"
            )
            || method.starts_with("window.");
        if !host_method {
            let details = method == "catalog.tool";
            let mut response = self.service.dispatch(request).await;
            if details {
                if let Some(tool) = &mut response.result {
                    self.icons.enrich_tool(
                        tool,
                        self.service.bootstrap()["catalog"]["revision"]
                            .as_u64()
                            .unwrap_or(0),
                    );
                }
            }
            return response;
        }
        let _serial = self.native_dispatch.lock().await;
        let fingerprint = serde_json::to_string(&(method, &request.params)).unwrap();
        if let Some((previous, response)) = self.completed_requests.lock().unwrap().get(&request.id)
        {
            return if previous == &fingerprint {
                response.clone()
            } else {
                Response::failure(
                    request.id,
                    CoreError::new("CONFLICT", "请求 ID 已用于其他参数。"),
                )
            };
        }
        let result = match method {
            "toolbox.downloadUpdate" => self.start_download(&request.params),
            "operation.retry" => self.start_download(&json!({"version":retry_version.unwrap()})),
            "toolbox.applyUpdate" => self.apply_update(&request.params, window.as_ref()).await,
            "window.minimize"
            | "window.toggleMaximize"
            | "window.startDragging"
            | "window.requestClose" => self.window_action(method, &request.params, window.as_ref()),
            _ => self
                .resolve_action(method, &request.params)
                .and_then(|action| {
                    match action {
                        NativeAction::OpenPath(path) => native::open(path.as_os_str()),
                        NativeAction::OpenUrl(url) => native::open(std::ffi::OsStr::new(&url)),
                        NativeAction::CopyText(text) => native::copy_text(&text),
                    }
                    .map_err(|error| CoreError::new("NATIVE_ACTION", error))?;
                    Ok(json!({"accepted":true}))
                }),
        };
        let response = match result {
            Ok(result) => Response::success(request.id.clone(), result),
            Err(error) => Response::failure(request.id.clone(), error),
        };
        if response.ok {
            self.completed_requests
                .lock()
                .unwrap()
                .insert(request.id, (fingerprint, response.clone()));
        }
        response
    }
    fn resolve_action(&self, method: &str, params: &Value) -> Result<NativeAction, CoreError> {
        if method == "system.openLink" {
            let url = match params["target"].as_str() {
                Some("release") => Some(self.release.lock().unwrap().page()),
                Some("homepage") => Some(release::PROJECT_URL.into()),
                Some("project") if params["toolId"].is_null() => Some(release::PROJECT_URL.into()),
                Some("feedback") => Some(format!("{}/issues", release::PROJECT_URL)),
                _ => None,
            };
            if let Some(url) = url {
                return Ok(NativeAction::OpenUrl(url));
            }
        }
        self.service.resolve_action(method, params)
    }
    fn window_action(
        &self,
        method: &str,
        params: &Value,
        window: Option<&tauri::WebviewWindow>,
    ) -> Result<Value, CoreError> {
        let window = window.ok_or_else(|| CoreError::new("NO_WINDOW", "此操作需要桌面窗口。"))?;
        let native_error = |error: tauri::Error| CoreError::new("NATIVE_ACTION", error.to_string());
        match method {
            "window.state" => {}
            "window.minimize" => {
                window.minimize().map_err(native_error)?;
            }
            "window.toggleMaximize" => {
                if window.is_maximized().map_err(native_error)? {
                    window.unmaximize().map_err(native_error)?;
                } else {
                    window.maximize().map_err(native_error)?;
                }
            }
            "window.startDragging" => {
                window.start_dragging().map_err(native_error)?;
            }
            "window.requestClose" => {
                if self.handoff_active.load(Ordering::Acquire) {
                    return Err(CoreError::new(
                        "BUSY",
                        "正在等待独立更新进程接手，完成后工具箱将自动重启。",
                    ));
                }
                if self.busy() && params["cancelTasks"] != true {
                    return Err(CoreError::new(
                        "BUSY",
                        "仍有任务正在执行，请继续等待，或取消任务后退出。",
                    ));
                }
                self.closing.store(true, Ordering::Relaxed);
                if params["cancelTasks"] == true {
                    self.cancel_all();
                }
                if !self.busy() {
                    self.exit_allowed.store(true, Ordering::Relaxed);
                    window.app_handle().exit(0);
                }
            }
            _ => return Err(CoreError::invalid("未知窗口操作。")),
        }
        Ok(
            json!({"accepted":true,"maximized":window.is_maximized().unwrap_or(false),"active":window.is_focused().unwrap_or(false)}),
        )
    }
    fn start_download(self: &Arc<Self>, params: &Value) -> Result<Value, CoreError> {
        let version = params["version"]
            .as_str()
            .ok_or_else(|| CoreError::invalid("需要指定工具箱版本。"))?;
        let Some(package) = self.release.lock().unwrap().package(version, VERSION) else {
            return Err(CoreError::new(
                "NO_UPDATE",
                "此版本没有可校验的 GitHub 更新包，请重新检查或查看发布页。",
            ));
        };
        self.verify_running_exe()?;
        let mut tasks = self.downloads.lock().unwrap();
        if let Some(task) = tasks.values().find(|task| {
            task.snapshot["version"] == version
                && matches!(
                    task.snapshot["state"].as_str(),
                    Some("queued" | "running" | "succeeded")
                )
        }) {
            return Ok(json!({"operationId":task.snapshot["id"]}));
        }
        let id = format!("update-{}", uuid::Uuid::new_v4());
        let cancel = Arc::new(AtomicBool::new(false));
        let operation = json!({"id":id,"kind":"toolboxUpdate","toolId":null,"version":version,"state":"queued","stage":"download","doneBytes":0,"totalBytes":package.size,"canCancel":true,"error":null,"result":null,"children":[]});
        tasks.insert(
            id.clone(),
            DownloadTask {
                snapshot: operation.clone(),
                cancel: cancel.clone(),
            },
        );
        drop(tasks);
        self.service.emit("operation.changed", operation);
        let host = self.clone();
        let version = version.to_owned();
        let operation_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let result = host
                .download(&operation_id, version, package, cancel.clone())
                .await;
            match result {
                Ok(mut tx) => {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = tx.rollback("用户取消了本体更新下载。");
                        host.change_download(
                            &operation_id,
                            json!({"state":"cancelled","stage":"complete","canCancel":false}),
                        );
                    } else {
                        host.change_download(&operation_id, json!({"state":"succeeded","stage":"complete","canCancel":false,"result":{"transactionId":tx.id,"readyToApply":true}}));
                    }
                }
                Err(error) => {
                    let cancelled = cancel.load(Ordering::Relaxed);
                    log(
                        host.service.workspace(),
                        "self-update-download-failed",
                        json!({"operationId":operation_id,"message":error}),
                    );
                    host.change_download(&operation_id, json!({"state":if cancelled {"cancelled"} else {"failed"},"stage":"download","canCancel":false,"error":{"code":if cancelled {"CANCELLED"} else if error.contains("HASH_MISMATCH") {"HASH_MISMATCH"} else {"NETWORK"},"message":error,"retryable":!cancelled,"operationId":operation_id}}));
                }
            }
        });
        Ok(json!({"operationId":id}))
    }
    async fn download(
        &self,
        id: &str,
        version: String,
        package: Package,
        cancel: Arc<AtomicBool>,
    ) -> Result<Transaction, String> {
        let directory = self.service.workspace().join("cache/self-update-downloads");
        self_update::ensure_managed(self.service.workspace(), &directory)?;
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let part = directory.join(format!("{id}.part"));
        self.change_download(id, json!({"state":"running","stage":"download"}));
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("PhysicalWorldToolbox/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(&package.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?;
        if response
            .content_length()
            .is_some_and(|size| size != package.size)
        {
            return Err("更新包大小与 GitHub 发布记录不一致。".into());
        }
        let mut file = tokio::fs::File::create_new(&part)
            .await
            .map_err(|e| e.to_string())?;
        let result = async {
            use tokio::io::AsyncWriteExt;
            let mut done = 0;
            let mut emitted = std::time::Instant::now();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                if cancel.load(Ordering::Relaxed) {
                    return Err("任务已取消。".into());
                }
                done += chunk.len() as u64;
                if done > package.size {
                    return Err("更新包大小与 GitHub 发布记录不一致。".into());
                }
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                if emitted.elapsed() >= Duration::from_millis(100) {
                    self.change_download(id, json!({"doneBytes":done}));
                    emitted = std::time::Instant::now();
                }
            }
            if done != package.size {
                return Err("更新包大小与 GitHub 发布记录不一致。".into());
            }
            file.sync_all().await.map_err(|e| e.to_string())?;
            drop(file);
            if cancel.load(Ordering::Relaxed) {
                return Err("任务已取消。".into());
            }
            self.change_download(id, json!({"doneBytes":done,"stage":"verify"}));
            let workspace = self.service.workspace().to_path_buf();
            let id = id.to_owned();
            let archive = part.clone();
            tauri::async_runtime::spawn_blocking(move || {
                Transaction::prepare(&workspace, &id, &version, &archive, &package.sha256)
            })
            .await
            .map_err(|e| e.to_string())?
        }
        .await;
        if self_update::ensure_managed(self.service.workspace(), &part).is_ok() {
            let _ = fs::remove_file(part);
        }
        result
    }
    fn verify_running_exe(&self) -> Result<(), CoreError> {
        let workspace = self.service.workspace();
        let target = workspace.join(self_update::EXE_NAME);
        let error = || {
            CoreError::new(
                "UNSUPPORTED_LAYOUT",
                "本体更新需要运行工作目录根部的 PhysicalWorldToolbox.exe。",
            )
        };
        self_update::ensure_managed(workspace, &target).map_err(|_| error())?;
        let expected = target.canonicalize().map_err(|_| error())?;
        if std::env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(CoreError::from)?
            == expected
        {
            Ok(())
        } else {
            Err(error())
        }
    }
    async fn apply_update(
        &self,
        params: &Value,
        window: Option<&tauri::WebviewWindow>,
    ) -> Result<Value, CoreError> {
        if params["confirmed"] != true {
            return Err(CoreError::invalid("应用本体更新需要确认。"));
        }
        if self.busy() {
            return Err(CoreError::new(
                "BUSY",
                "请等待安装或下载任务结束后再应用本体更新。",
            ));
        }
        let id = params["operationId"]
            .as_str()
            .ok_or_else(|| CoreError::invalid("需要更新任务 ID。"))?;
        let tx = Transaction::load(self.service.workspace(), id)
            .map_err(|e| CoreError::new("RECOVERY_REQUIRED", e))?;
        if tx.stage != Stage::Prepared || !release::newer(&tx.version, VERSION) {
            return Err(CoreError::new(
                "RECOVERY_REQUIRED",
                "更新任务尚未准备好，请重新下载或重启恢复。",
            ));
        }
        self.verify_running_exe()?;
        self.accept_handoff(move || self_update::handoff(&tx))
            .await?;
        if let Some(window) = window {
            window.app_handle().exit(0);
        }
        Ok(json!({"accepted":true,"restarting":true}))
    }
    fn should_exit_after_tasks(&self) -> bool {
        self.closing.load(Ordering::Relaxed) && !self.busy()
    }
    async fn accept_handoff(
        &self,
        execute: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Result<(), CoreError> {
        if self
            .handoff_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(CoreError::new("BUSY", "本体更新正在交接。"));
        }
        let _handoff = HandoffFlag(&self.handoff_active);
        let result = tauri::async_runtime::spawn_blocking(execute)
            .await
            .map_err(|error| CoreError::new("UPDATE_HANDOFF", error.to_string()))?;
        if let Err(error) = result {
            return Err(CoreError::new("UPDATE_HANDOFF", error));
        }
        self.exit_allowed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

struct HandoffFlag<'a>(&'a AtomicBool);
impl Drop for HandoffFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn download_candidates(url: &str, settings: &Value) -> Vec<String> {
    let mut result = vec![];
    let github = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            [
                "github.com",
                "raw.githubusercontent.com",
                "objects.githubusercontent.com",
                "release-assets.githubusercontent.com",
            ]
            .contains(&host.as_str())
        });
    if github && settings["githubProxy"]["enabled"] == true {
        if let Some(proxies) = settings["githubProxy"]["urls"].as_array() {
            for proxy in proxies.iter().filter_map(Value::as_str) {
                let encoded: String =
                    url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
                let candidate = if proxy.contains("{encoded_url}") {
                    proxy.replace("{encoded_url}", &encoded)
                } else if proxy.contains("{url}") {
                    proxy.replace("{url}", url)
                } else {
                    format!(
                        "{}{url}",
                        if proxy.ends_with('/') {
                            proxy.to_owned()
                        } else {
                            format!("{proxy}/")
                        }
                    )
                };
                if !result.contains(&candidate) {
                    result.push(candidate);
                }
            }
        }
    }
    result.push(url.to_owned());
    result
}

#[tauri::command]
async fn toolbox_dispatch(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<HostState>>,
    request: Request,
) -> Result<Response, String> {
    if window.label() != "main" {
        return Ok(Response::failure(
            request.id,
            CoreError::new("FORBIDDEN", "此窗口不能调用工具箱服务。"),
        ));
    }
    Ok(state.inner().clone().dispatch(request, Some(window)).await)
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
pub fn log(workspace: &Path, event: &str, data: Value) {
    let directory = workspace.join("cache/logs");
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(directory.join("host.jsonl"))
    {
        let _ = writeln!(
            file,
            "{}",
            json!({"unixMs":unix_ms(),"pid":std::process::id(),"event":event,"data":data})
        );
    }
}

pub fn run() -> Result<(), String> {
    let arguments = cli::parse(std::env::args().skip(1))?;
    let workspace = match &arguments.workspace {
        Some(path) => path.clone(),
        None => std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("无法定位工具箱目录。")?
            .to_path_buf(),
    };
    if !workspace.exists() {
        fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    }
    let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
    match &arguments.mode {
        Mode::ApplyUpdate(id) => {
            let result = self_update::run_helper(&workspace, id, arguments.parent_pid.unwrap());
            log(
                &workspace,
                "update-helper-exit",
                json!({"transactionId":id,"result":result}),
            );
            return result;
        }
        Mode::Diagnose | Mode::Request(_) => return headless(arguments, &workspace),
        Mode::Gui => {}
    }
    if arguments.trial.is_none() {
        let upgrade = self_update::upgrade_lock(&workspace)?
            .ok_or("工具箱正在应用本体更新，请稍后再打开。")?;
        let pending = self_update::pending_recovery(&workspace)?;
        drop(upgrade);
        if let Some(tx) = pending {
            self_update::handoff(&tx)?;
            return Ok(());
        }
    } else {
        let tx = Transaction::load(&workspace, arguments.trial.as_deref().unwrap())?;
        if tx.stage != Stage::AwaitingReady {
            return Err("本体试运行事务已失效。".into());
        }
    }
    let Some(instance) = InstanceGuard::acquire(&workspace)? else {
        let _ = fs::write(
            workspace.join("cache/activate.request"),
            std::process::id().to_string(),
        );
        return Ok(());
    };
    if tauri::webview_version().is_err() {
        loop {
            if !native::message("工具箱需要 Microsoft Edge WebView2 Evergreen Runtime。\n\n是否打开微软官方下载页？安装后再次打开工具箱即可。", true) { return Ok(()); }
            native::open(std::ffi::OsStr::new(
                "https://developer.microsoft.com/microsoft-edge/webview2/",
            ))?;
            if !native::message(
                "完成 WebView2 安装后，选择“是”重新检测；选择“否”退出。",
                true,
            ) {
                return Ok(());
            }
            if tauri::webview_version().is_ok() {
                break;
            }
        }
    }
    let service = if arguments.trial.is_some() {
        Service::open_read_only(&workspace, VERSION)
    } else {
        Service::open(&workspace, VERSION)
    }
    .map_err(|e| e.to_string())?;
    let host = HostState::new(service, arguments.trial, true);
    log(
        &workspace,
        "startup",
        json!({"version":VERSION,"embeddedFrontend":cfg!(feature="custom-protocol"),"workspace":workspace}),
    );
    let setup_host = host.clone();
    let app = tauri::Builder::default()
        .manage(host.clone())
        .invoke_handler(tauri::generate_handler![toolbox_dispatch])
        .setup(move |app| {
            let profile = setup_host.service.workspace().join("cache/webview");
            self_update::ensure_managed(setup_host.service.workspace(), &profile).map_err(std::io::Error::other)?;
            fs::create_dir_all(&profile)?;
            let window = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .data_directory(profile)
                .on_navigation(|url| url.scheme() == "tauri" && url.host_str() == Some("localhost") || matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            if let Ok(Some(monitor)) = window.current_monitor() {
                let scale = monitor.scale_factor();
                let width = (f64::from(monitor.size().width) / scale - 64.0).clamp(640.0, 1420.0);
                let height = (f64::from(monitor.size().height) / scale - 80.0).clamp(440.0, 900.0);
                let _ = window.set_size(tauri::LogicalSize::new(width, height));
                let _ = window.center();
            }
            window.show()?;
            let handle = app.handle().clone();
            let icon_host = setup_host.clone();
            let mut events = setup_host.service.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(mut event) => {
                            if event.event == "catalog.changed" {
                                icon_host.enrich_catalog(&mut event.data);
                                icon_host.schedule_icons(&event.data, &icon_host.service.bootstrap()["settings"]);
                            } else if event.event == "toolbox.changed" {
                                event.data = icon_host.toolbox();
                            }
                            let _ = handle.emit_to("main", "toolbox://event", event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            let monitor_host = setup_host.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let started = std::time::Instant::now();
                let mut reported = false;
                let mut trial_committed = false;
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    if !trial_committed && monitor_host.trial.is_some() && !monitor_host.trial_pending() {
                        match monitor_host.service.enable_writes_after_trial() {
                            Ok(()) => {
                                trial_committed = true;
                                let toolbox = monitor_host.toolbox();
                                monitor_host.service.emit("toolbox.changed", toolbox);
                                log(monitor_host.service.workspace(), "self-update-committed", json!({"transactionId":monitor_host.trial}));
                            }
                            Err(error) => log(monitor_host.service.workspace(), "trial-enable-writes-error", json!({"message":error.to_string()})),
                        }
                    }
                    let activation = monitor_host.service.workspace().join("cache/activate.request");
                    if activation.is_file() {
                        let _ = fs::remove_file(activation);
                        if let Some(window) = handle.get_webview_window("main") { let _ = window.show(); let _ = window.unminimize(); let _ = window.set_focus(); }
                    }
                    if monitor_host.should_exit_after_tasks() { monitor_host.exit_allowed.store(true, Ordering::Relaxed); handle.exit(0); break; }
                    if !reported && !monitor_host.ready.load(Ordering::Relaxed) && started.elapsed() > Duration::from_secs(25) {
                        reported = true;
                        log(monitor_host.service.workspace(), "frontend-ready-timeout", json!({"message":"WebView 首屏未确认"}));
                        let _ = tauri::async_runtime::spawn_blocking(|| native::message("工具箱界面未能完成初始化。请关闭后重试；详细信息在工作目录的 cache\\logs 中。", false)).await;
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            let state = window.state::<Arc<HostState>>();
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } if state.busy() && !state.exit_allowed.load(Ordering::Relaxed) => {
                    api.prevent_close();
                    state.service.emit("window.changed", json!({"closeRequested":true,"busy":true,"canCancel":true}));
                }
                tauri::WindowEvent::Resized(_) => state.service.emit("window.changed", json!({"maximized":window.is_maximized().unwrap_or(false),"active":window.is_focused().unwrap_or(false)})),
                tauri::WindowEvent::Focused(active) => state.service.emit("window.changed", json!({"maximized":window.is_maximized().unwrap_or(false),"active":active})),
                _ => {}
            }
        })
        .build(tauri::generate_context!()).map_err(|e| e.to_string())?;
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if host.busy() && !host.exit_allowed.load(Ordering::Relaxed) {
                api.prevent_exit();
                host.service.emit(
                    "window.changed",
                    json!({"closeRequested":true,"busy":true,"canCancel":true}),
                );
            }
            let _ = handle;
        }
        let _keep_lock_alive = &instance;
    });
    Ok(())
}

fn headless(arguments: Arguments, workspace: &Path) -> Result<(), String> {
    let mut upgrade =
        Some(self_update::upgrade_lock(workspace)?.ok_or("BUSY: 本体更新或试运行正在进行。")?);
    if !matches!(arguments.mode, Mode::Diagnose)
        && self_update::pending_recovery(workspace)?.is_some()
    {
        return Err("RECOVERY_REQUIRED: 请先正常启动工具箱，恢复上次未完成的本体更新。".into());
    }
    let Some(_instance) = InstanceGuard::acquire(workspace)? else {
        return Err("BUSY: 此工作区的工具箱已经运行。".into());
    };
    let service = if matches!(arguments.mode, Mode::Diagnose) {
        Service::open_read_only(workspace, VERSION)
    } else {
        Service::open(workspace, VERSION)
    }
    .map_err(|e| e.to_string())?;
    let host = HostState::new(service, None, false);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let result = runtime.block_on(async {
        match arguments.mode {
            Mode::Diagnose => Ok::<Value, String>(host.snapshot()),
            Mode::Request(path) => {
                let request: Request =
                    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                // The helper must acquire the upgrade lock before acknowledging a handoff.
                // Our instance lock continues to exclude every other ordinary caller.
                if request.method == "toolbox.applyUpdate" {
                    drop(upgrade.take());
                }
                let response = host.dispatch(request, None).await;
                if arguments.wait {
                    let started = std::time::Instant::now();
                    while host.busy() {
                        if started.elapsed() > Duration::from_secs(180) {
                            host.cancel_all();
                            return Err("诊断任务等待超时。".into());
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Ok(json!({"response":response,"bootstrap":host.snapshot()}))
            }
            _ => unreachable!(),
        }
    })?;
    let bytes = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    if let Some(output) = arguments.output {
        fs::write(output, bytes).map_err(|e| e.to_string())?;
    } else {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_chrome_keeps_native_resize_and_shadow_without_a_caption() {
        let config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let window = &config.app.windows[0];
        assert!(
            !window.decorations,
            "the frontend owns the embedded title area"
        );
        assert!(window.resizable && window.shadow);
        assert!(window.minimizable && window.maximizable && window.closable);
    }
    #[test]
    fn embedded_drag_permissions_do_not_bypass_the_guarded_close_command() {
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        let window_permissions: Vec<_> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|permission| permission.starts_with("core:window:"))
            .collect();
        assert_eq!(
            window_permissions,
            [
                "core:window:allow-start-dragging",
                "core:window:allow-internal-toggle-maximize"
            ]
        );
    }
    #[tokio::test]
    async fn window_state_requires_a_desktop_window() {
        let host = empty_host();
        let response = host
            .dispatch(
                Request {
                    v: 1,
                    id: "headless-window-state".into(),
                    method: "window.state".into(),
                    params: json!({}),
                },
                None,
            )
            .await;
        assert_eq!(response.error.unwrap().code, "NO_WINDOW");
    }
    fn empty_host() -> Arc<HostState> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.work/host-tests")
            .join(format!("handoff-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        HostState::new(Service::open(&root, VERSION).unwrap(), None, false)
    }
    #[tokio::test]
    async fn a_slow_handoff_freezes_writes_without_allowing_the_monitor_to_exit() {
        let host = empty_host();
        let worker = host.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            worker
                .accept_handoff(move || {
                    let _ = entered_tx.send(());
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
        });
        entered_rx.await.unwrap();
        let premature_exit = host.should_exit_after_tasks();
        let rejected = host
            .dispatch(
                Request {
                    v: 1,
                    id: "during-handoff".into(),
                    method: "settings.update".into(),
                    params: json!({"revision":1,"patch":{}}),
                },
                None,
            )
            .await;
        release_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(!premature_exit, "the monitor must wait for helper.accepted");
        assert_eq!(rejected.error.unwrap().code, "BUSY");
        assert!(host.exit_allowed.load(Ordering::Relaxed));
    }
    #[tokio::test]
    async fn a_failed_handoff_keeps_the_host_running_and_restores_writes() {
        let host = empty_host();
        host.accept_handoff(|| Err("controlled handoff failure".into()))
            .await
            .unwrap_err();
        assert!(!host.should_exit_after_tasks());
        assert!(!host.exit_allowed.load(Ordering::Relaxed));
        let response = host
            .dispatch(
                Request {
                    v: 1,
                    id: "after-failure".into(),
                    method: "settings.update".into(),
                    params: json!({"revision":1,"patch":{"suppressDirectoryWarning":true}}),
                },
                None,
            )
            .await;
        assert!(response.ok);
    }
    #[tokio::test]
    async fn bootstrap_and_details_share_the_real_package_icon() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.work/host-tests")
            .join(format!("ipc-icons-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("config/registry-cache/tools")).unwrap();
        fs::create_dir_all(root.join("tools/tool-one")).unwrap();
        fs::write(root.join("config/registry-cache/index.json"), json!({"schemaVersion":1,"tools":[{"id":"tool-one","name":"图标测试","manifestUrl":"tools/tool-one.json"}]}).to_string()).unwrap();
        fs::write(root.join("config/registry-cache/tools/tool-one.json"), json!({"schemaVersion":1,"id":"tool-one","name":"图标测试","entry":"bin/run.cmd","iconSource":"icon.ico","latest":{"stable":"1.0.0"},"versions":[{"version":"1.0.0","packageUrl":"http://127.0.0.1/package.zip","sha256":"0".repeat(64)}]}).to_string()).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.ico"),
            root.join("tools/tool-one/icon.ico"),
        )
        .unwrap();
        let host = HostState::new(Service::open(&root, VERSION).unwrap(), None, false);
        let bootstrap = host
            .dispatch(
                Request {
                    v: 1,
                    id: "bootstrap".into(),
                    method: "app.bootstrap".into(),
                    params: json!({}),
                },
                None,
            )
            .await;
        let detail = host
            .dispatch(
                Request {
                    v: 1,
                    id: "details".into(),
                    method: "catalog.tool".into(),
                    params: json!({"toolId":"tool-one"}),
                },
                None,
            )
            .await;
        let icon = bootstrap.result.as_ref().unwrap()["catalog"]["tools"][0]["iconUrl"]
            .as_str()
            .unwrap();
        assert!(icon.starts_with("data:image/x-icon;base64,"));
        assert_eq!(
            icon,
            detail.result.as_ref().unwrap()["iconUrl"].as_str().unwrap()
        );
    }
}
