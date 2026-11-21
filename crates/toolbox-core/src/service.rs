use crate::{
    catalog::{load_installed, Catalog},
    model::*,
    network,
    paths::{atomic_json, managed, read_json, validate_id},
    settings::{Settings, PROXIES},
    transaction,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::broadcast;

pub struct Service {
    workspace: PathBuf,
    app_version: String,
    session_id: String,
    state: Mutex<State>,
    events: broadcast::Sender<EventEnvelope>,
    commands: Mutex<()>,
    writer: Mutex<()>,
    processes: Mutex<BTreeMap<String, Vec<Child>>>,
    read_only: AtomicBool,
}
struct State {
    catalog: Catalog,
    settings: Settings,
    preferences: Value,
    seq: u64,
    operations: BTreeMap<String, Operation>,
    cancellations: BTreeMap<String, Arc<AtomicBool>>,
    dedupe: BTreeMap<String, (String, Response)>,
}
#[derive(Clone)]
struct Plan {
    kind: String,
    id: String,
    manifest: Option<Manifest>,
    version: Option<ToolVersion>,
}
impl Service {
    pub fn open(workspace: impl AsRef<Path>, app_version: &str) -> CoreResult<Arc<Self>> {
        Self::open_options(workspace.as_ref(), app_version, false)
    }
    pub fn open_read_only(workspace: impl AsRef<Path>, app_version: &str) -> CoreResult<Arc<Self>> {
        Self::open_options(workspace.as_ref(), app_version, true)
    }
    fn open_options(workspace: &Path, app_version: &str, read_only: bool) -> CoreResult<Arc<Self>> {
        // Strip the verbatim prefix before passing cwd to cmd.exe. Rust still supports long file paths.
        let canonical = fs::canonicalize(workspace)?;
        #[cfg(windows)]
        let workspace = {
            let text = canonical.to_string_lossy();
            if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
                PathBuf::from(format!("\\\\{unc}"))
            } else if let Some(path) = text.strip_prefix("\\\\?\\") {
                PathBuf::from(path)
            } else {
                canonical
            }
        };
        #[cfg(not(windows))]
        let workspace = canonical;
        crate::paths::reject_link(&workspace)?;
        let settings = Settings::load(&workspace)?;
        let recovery = if read_only {
            BTreeMap::new()
        } else {
            transaction::recover(&workspace)
        };
        let mut catalog = Catalog::load(&workspace);
        catalog.recovery = recovery.keys().cloned().collect();
        if let Some(message) = recovery.get("*") {
            catalog.installed_error = Some(message.clone());
        }
        if !recovery.is_empty() {
            catalog.error = Some(format!(
                "部分工具需要恢复：{}",
                recovery.values().cloned().collect::<Vec<_>>().join("；")
            ));
        }
        let mut preferences = read_json(&workspace.join("config/launcher-ui.json"))
            .ok()
            .filter(|p| p["schemaVersion"] == 1 && p["pinnedToolIds"].is_array())
            .unwrap_or_else(
                || json!({"schemaVersion":1,"pinnedToolIds":[],"lastSelectedToolId":null}),
            );
        preferences["revision"] = json!(1);
        if preferences["lastSelectedToolId"].as_str().is_none_or(|id| {
            !catalog.manifests.contains_key(id) && !catalog.installed.contains_key(id)
        }) {
            let selected = if catalog.manifests.contains_key("dnf_palette_prism") {
                Some("dnf_palette_prism".to_string())
            } else {
                catalog
                    .installed
                    .keys()
                    .next()
                    .cloned()
                    .or_else(|| catalog.index.tools.first().map(|t| t.id.clone()))
            };
            preferences["lastSelectedToolId"] = json!(selected);
        }
        let mut operations = BTreeMap::new();
        if let Ok(entries) = fs::read_dir(workspace.join("cache/operations")) {
            for entry in entries.flatten() {
                if let Ok(value) = read_json(&entry.path().join("operation.json")) {
                    if let Ok(mut operation) = serde_json::from_value::<Operation>(value) {
                        if operation.active() {
                            reconcile_interrupted(&mut operation, &workspace, &catalog.recovery);
                            if !read_only {
                                let _ =
                                    atomic_json(&entry.path().join("operation.json"), &operation);
                            }
                        }
                        operations.insert(operation.id.clone(), operation);
                    }
                }
            }
        }
        let (events, _) = broadcast::channel(256);
        let service = Arc::new(Self {
            workspace,
            app_version: app_version.into(),
            session_id: uuid::Uuid::new_v4().to_string(),
            state: Mutex::new(State {
                catalog,
                settings,
                preferences,
                seq: 0,
                operations,
                cancellations: BTreeMap::new(),
                dedupe: BTreeMap::new(),
            }),
            events,
            commands: Mutex::new(()),
            writer: Mutex::new(()),
            processes: Mutex::new(BTreeMap::new()),
            read_only: AtomicBool::new(read_only),
        });
        if !read_only {
            service.prune_operations();
        }
        Ok(service)
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn session_id(&self) -> String {
        self.session_id.clone()
    }
    pub fn enable_writes_after_trial(&self) -> CoreResult<()> {
        let _command = self.commands.lock().unwrap();
        if !self.read_only.load(Ordering::Acquire) {
            return Ok(());
        }
        let recovery = transaction::recover(&self.workspace);
        let mut state = self.state.lock().unwrap();
        let mut catalog = Catalog::load(&self.workspace);
        catalog.revision = state.catalog.revision + 1;
        catalog.recovery = recovery.keys().cloned().collect();
        if !recovery.is_empty() {
            catalog.error = Some(recovery.values().cloned().collect::<Vec<_>>().join("；"));
        }
        state.catalog = catalog;
        self.read_only.store(false, Ordering::Release);
        let view = self.catalog_view(&state);
        self.emit_locked(&mut state, "catalog.changed", view);
        drop(state);
        self.prune_operations();
        Ok(())
    }
    pub fn is_busy(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .operations
            .values()
            .any(Operation::active)
    }
    pub fn cancel_cancellable(&self) -> usize {
        let state = self.state.lock().unwrap();
        let mut count = 0;
        for operation in state
            .operations
            .values()
            .filter(|op| op.active() && op.can_cancel)
        {
            if let Some(cancel) = state.cancellations.get(&operation.id) {
                cancel.store(true, Ordering::Relaxed);
                count += 1;
            }
        }
        count
    }
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.events.subscribe()
    }
    pub fn emit(&self, event: &str, data: Value) {
        self.emit_locked(&mut self.state.lock().unwrap(), event, data);
    }
    fn emit_locked(&self, state: &mut State, event: &str, data: Value) {
        state.seq += 1;
        let _ = self.events.send(EventEnvelope {
            v: 1,
            session_id: self.session_id.clone(),
            seq: state.seq,
            event: event.into(),
            data,
        });
    }
    fn busy_ids(state: &State, include_queued: bool) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        for op in state
            .operations
            .values()
            .filter(|o| o.active() && (include_queued || o.state == "running"))
        {
            if let Some(id) = &op.tool_id {
                ids.insert(id.clone());
            }
            for child in &op.children {
                if (include_queued
                    && ["queued", "running"].contains(&child["state"].as_str().unwrap_or("")))
                    || child["state"] == "running"
                {
                    if let Some(id) = child["toolId"].as_str() {
                        ids.insert(id.into());
                    }
                }
            }
        }
        ids
    }
    fn catalog_view(&self, state: &State) -> Value {
        state.catalog.view(
            &self.workspace,
            &state.settings,
            &self.app_version,
            &Self::busy_ids(state, false),
        )
    }
    pub fn bootstrap(&self) -> Value {
        let state = self.state.lock().unwrap();
        let catalog = self.catalog_view(&state);
        let directory_warning = directory_warning(
            &self.workspace,
            state.settings.raw["suppressDirectoryWarning"] == true,
        );
        let read_only = self.read_only.load(Ordering::Acquire);
        json!({"protocol":1,"sessionId":self.session_id,"seq":state.seq,"catalog":catalog,"preferences":state.preferences,"settings":state.settings.view(),"operations":state.operations.values().collect::<Vec<_>>(),"toolbox":state.catalog.toolbox(&self.app_version),"about":about(),"directoryWarning":directory_warning,"capabilities":{"install":!read_only&&state.catalog.installed_error.is_none(),"launch":!read_only,"selfUpdate":!read_only,"cancel":true,"openWorkspace":true}})
    }
    pub async fn dispatch(self: &Arc<Self>, request: Request) -> Response {
        if request.v != 1 {
            return Response::failure(
                request.id,
                CoreError::new("UNSUPPORTED_VERSION", "仅支持桥接协议 v1"),
            );
        }
        if request.id.is_empty()
            || request.id.len() > 128
            || !request.params.is_null() && !request.params.is_object()
        {
            return Response::failure(request.id, CoreError::invalid("请求 ID 或参数格式无效"));
        }
        let _command = self.commands.lock().unwrap();
        let fingerprint = format!("{}:{}", request.method, request.params);
        if let Some((original, response)) = self.state.lock().unwrap().dedupe.get(&request.id) {
            return if original == &fingerprint {
                response.clone()
            } else {
                Response::failure(request.id, CoreError::invalid("同一请求 ID 已用于不同参数"))
            };
        }
        let read_method = matches!(
            request.method.as_str(),
            "app.bootstrap"
                | "app.ready"
                | "catalog.tool"
                | "operation.list"
                | "settings.get"
                | "settings.proxyDefaults"
        );
        let result = if self.read_only.load(Ordering::Acquire) && !read_method {
            Err(CoreError::new("BUSY", "本体更新尚未完成健康确认"))
        } else {
            self.handle(&request)
        };
        let response = match result {
            Ok(value) => Response::success(request.id.clone(), value),
            Err(error) => Response::failure(request.id.clone(), error),
        };
        if !read_method && response.ok {
            self.state
                .lock()
                .unwrap()
                .dedupe
                .insert(request.id, (fingerprint, response.clone()));
        }
        response
    }
    fn handle(self: &Arc<Self>, request: &Request) -> CoreResult<Value> {
        let params = &request.params;
        match request.method.as_str() {
            "app.bootstrap" => Ok(self.bootstrap()),
            "app.ready" => Ok(json!({"accepted":request.params["sessionId"] == self.session_id})),
            "settings.get" => Ok(self.state.lock().unwrap().settings.view()),
            "settings.proxyDefaults" => Ok(json!({"enabled":true,"urls":PROXIES})),
            "settings.update" => {
                let mut state = self.state.lock().unwrap();
                if params["revision"].as_u64() != Some(state.settings.revision) {
                    return Err(CoreError::new(
                        "STALE_REVISION",
                        format!(
                            "设置已改变，请重新加载。当前修订号：{}",
                            state.settings.revision
                        ),
                    ));
                }
                state.settings.update(&self.workspace, &params["patch"])?;
                let view = state.settings.view();
                self.emit_locked(&mut state, "settings.changed", view.clone());
                let catalog = self.catalog_view(&state);
                self.emit_locked(&mut state, "catalog.changed", catalog);
                Ok(view)
            }
            "catalog.tool" => {
                let id = required(params, "toolId")?;
                let state = self.state.lock().unwrap();
                ensure_tool(&state, id)?;
                Ok(state.catalog.tool(
                    id,
                    &self.workspace,
                    &state.settings,
                    &self.app_version,
                    &Self::busy_ids(&state, false),
                ))
            }
            "catalog.refresh" => self.start_refresh(params),
            "preferences.setPinned" | "preferences.selectTool" => {
                let id = required(params, "toolId")?;
                let mut state = self.state.lock().unwrap();
                ensure_tool(&state, id)?;
                let mut preferences = state.preferences.clone();
                if request.method == "preferences.selectTool" {
                    preferences["lastSelectedToolId"] = json!(id);
                } else {
                    let pinned = params["pinned"]
                        .as_bool()
                        .ok_or_else(|| CoreError::invalid("pinned 必须为布尔值"))?;
                    let ids = preferences["pinnedToolIds"].as_array_mut().unwrap();
                    ids.retain(|value| value != id);
                    if pinned {
                        ids.push(json!(id));
                    }
                }
                preferences["revision"] =
                    json!(state.preferences["revision"].as_u64().unwrap_or(0) + 1);
                let mut disk = preferences.clone();
                disk.as_object_mut().unwrap().remove("revision");
                atomic_json(&managed(&self.workspace, "config/launcher-ui.json")?, &disk)?;
                state.preferences = preferences.clone();
                self.emit_locked(&mut state, "preferences.changed", preferences.clone());
                Ok(preferences)
            }
            "operation.start" => self.start_operation(params),
            "operation.list" => Ok(json!(self
                .state
                .lock()
                .unwrap()
                .operations
                .values()
                .collect::<Vec<_>>())),
            "operation.cancel" => {
                let id = required(params, "operationId")?;
                let state = self.state.lock().unwrap();
                let operation = state
                    .operations
                    .get(id)
                    .ok_or_else(|| CoreError::invalid("任务不存在"))?;
                if !operation.active() {
                    return Ok(json!({"accepted":false,"state":operation.state}));
                }
                if !operation.can_cancel {
                    return Err(CoreError::new("BUSY", "文件正在提交或恢复，请等待完成"));
                }
                if let Some(cancel) = state.cancellations.get(id) {
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(json!({"accepted":true}))
            }
            "operation.retry" => {
                let id = required(params, "operationId")?;
                let operation = self
                    .state
                    .lock()
                    .unwrap()
                    .operations
                    .get(id)
                    .cloned()
                    .ok_or_else(|| CoreError::invalid("任务不存在"))?;
                if operation.active() {
                    return Err(CoreError::new("BUSY", "任务仍在进行"));
                }
                if operation.kind == "refresh" {
                    return self.start_refresh(&json!({"reason":"manual"}));
                }
                let mut retry = operation.request.clone();
                retry.as_object_mut().unwrap().remove("revision");
                if operation.kind == "updateAll" {
                    retry["toolIds"] = json!(operation
                        .children
                        .iter()
                        .filter(|child| child["state"] == "failed" || child["state"] == "cancelled")
                        .map(|child| child["toolId"].clone())
                        .collect::<Vec<_>>());
                }
                self.start_operation(&retry)
            }
            "tool.launch" => {
                self.launch(required(params, "toolId")?)?;
                Ok(json!({"accepted":true}))
            }
            _ => Err(CoreError::invalid(format!(
                "未知业务方法：{}",
                request.method
            ))),
        }
    }
    fn create_operation(
        &self,
        state: &mut State,
        kind: &str,
        mut params: Value,
        tool_id: Option<String>,
        version: Option<String>,
        mut children: Vec<Value>,
    ) -> CoreResult<(String, Arc<AtomicBool>)> {
        params["sessionId"] = json!(self.session_id);
        let id = format!("op-{}", uuid::Uuid::new_v4().simple());
        for (index, child) in children.iter_mut().enumerate() {
            child["transactionId"] = json!(format!("{id}-i{index}"));
        }
        let timestamp = now();
        let operation = Operation {
            id: id.clone(),
            kind: kind.into(),
            tool_id,
            version,
            state: "queued".into(),
            stage: "queued".into(),
            done_bytes: 0,
            total_bytes: None,
            can_cancel: true,
            error: None,
            children,
            result: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            request: params,
        };
        let root = managed(
            &self.workspace,
            &format!("cache/operations/{id}/operation.json"),
        )?;
        atomic_json(&root, &operation)?;
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancellations.insert(id.clone(), cancel.clone());
        state.operations.insert(id.clone(), operation.clone());
        self.emit_locked(state, "operation.changed", serde_json::to_value(operation)?);
        Ok((id, cancel))
    }
    fn start_refresh(self: &Arc<Self>, params: &Value) -> CoreResult<Value> {
        let mut state = self.state.lock().unwrap();
        if let Some(operation) = state.operations.values().find(|op| {
            op.kind == "refresh"
                && op.active()
                && op.request["source"] == state.settings.index_url()
                && op.request["channel"] == state.settings.channel()
        }) {
            return Ok(json!({"operationId":operation.id}));
        }
        let settings = state.settings.clone();
        let request = json!({"reason":params["reason"],"source":settings.index_url(),"channel":settings.channel()});
        let (id, cancel) =
            self.create_operation(&mut state, "refresh", request, None, None, vec![])?;
        let service = Arc::clone(self);
        let operation_id = id.clone();
        std::thread::spawn(move || {
            service.change_operation(
                &operation_id,
                |op| {
                    op.state = "running".into();
                    op.stage = "fetchIndex".into();
                },
                true,
            );
            let result = network::refresh(&service.workspace, &settings, &cancel).and_then(
                |(index, manifests, checked, generation)| {
                    let mut state = service.state.lock().unwrap();
                    if state.settings.index_url() != settings.index_url()
                        || state.settings.channel() != settings.channel()
                    {
                        return Err(CoreError::new(
                            "STALE_REVISION",
                            "检查期间源或渠道已改变，请再次检查",
                        ));
                    }
                    network::check_cancel(&cancel)?;
                    network::publish_generation(&service.workspace, &generation)?;
                    let mut local = Catalog::load(&service.workspace);
                    local.index = index;
                    for (id, manifest) in manifests {
                        local.manifests.insert(id, manifest);
                    }
                    local.source = "fresh".into();
                    local.checked_at = Some(checked);
                    local.revision = state.catalog.revision + 1;
                    local.recovery = state.catalog.recovery.clone();
                    state.catalog = local;
                    let view = service.catalog_view(&state);
                    let toolbox = state.catalog.toolbox(&service.app_version);
                    service.emit_locked(&mut state, "catalog.changed", view);
                    service.emit_locked(&mut state, "toolbox.changed", toolbox);
                    Ok(json!({"revision":state.catalog.revision}))
                },
            );
            if let Err(error) = &result {
                let mut state = service.state.lock().unwrap();
                state.catalog.error = Some(error.message.clone());
                let view = service.catalog_view(&state);
                service.emit_locked(&mut state, "catalog.changed", view);
            }
            service.finish_operation(&operation_id, result);
        });
        Ok(json!({"operationId":id}))
    }
    fn plan(&self, state: &State, kind: &str, id: &str, version: Option<&str>) -> CoreResult<Plan> {
        ensure_tool(state, id)?;
        if let Some(error) = &state.catalog.installed_error {
            return Err(CoreError::new("RECOVERY_REQUIRED", error));
        }
        if state.catalog.recovery.contains(id) {
            return Err(CoreError::new(
                "RECOVERY_REQUIRED",
                "此工具的上次事务需要恢复",
            ));
        }
        if Self::busy_ids(state, true).contains(id) {
            return Err(CoreError::new("BUSY", "此工具已有待执行任务"));
        }
        let installed = state.catalog.installed.get(id);
        if kind == "uninstall" {
            if installed.is_none() {
                return Err(CoreError::new("NOT_INSTALLED", "此工具尚未安装"));
            }
            return Ok(Plan {
                kind: kind.into(),
                id: id.into(),
                manifest: None,
                version: None,
            });
        }
        if ["update", "reinstall", "switchVersion"].contains(&kind) && installed.is_none() {
            return Err(CoreError::new("NOT_INSTALLED", "此工具尚未安装，请先安装"));
        }
        let manifest =
            state.catalog.manifests.get(id).ok_or_else(|| {
                CoreError::new("INCOMPATIBLE", "工具清单不可用，请先检查工具列表")
            })?;
        let selected = manifest.selected(state.settings.channel(), version, &self.app_version)?;
        if kind == "install" && installed.is_some_and(|old| old.version == selected.version) {
            return Err(CoreError::invalid(
                "此版本已安装；需要覆盖文件时请使用重新安装",
            ));
        }
        if kind == "update"
            && installed.is_some_and(|old| {
                crate::compare_versions(&selected.version, &old.version)
                    != std::cmp::Ordering::Greater
            })
        {
            return Err(CoreError::invalid("当前已是最新版本"));
        }
        Ok(Plan {
            kind: kind.into(),
            id: id.into(),
            manifest: Some(manifest.clone()),
            version: Some(selected),
        })
    }
    fn start_operation(self: &Arc<Self>, params: &Value) -> CoreResult<Value> {
        let kind = required(params, "kind")?;
        if ![
            "install",
            "update",
            "reinstall",
            "switchVersion",
            "uninstall",
            "updateAll",
        ]
        .contains(&kind)
        {
            return Err(CoreError::invalid("不支持此任务类型"));
        }
        if kind == "uninstall" && params["confirmed"] != true {
            return Err(CoreError::new(
                "CONFIRMATION_REQUIRED",
                "卸载会移除此工具整个目录，包括 config 与 data；请在确认页确认",
            ));
        }
        let mut state = self.state.lock().unwrap();
        if params.get("revision").is_some()
            && params["revision"].as_u64() != Some(state.catalog.revision)
        {
            return Err(CoreError::new(
                "STALE_REVISION",
                format!(
                    "工具列表已改变，请重新加载。当前修订号：{}",
                    state.catalog.revision
                ),
            ));
        }
        let mut plans = vec![];
        if kind == "updateAll" {
            let subset = params.get("toolIds").and_then(Value::as_array);
            for (id, old) in &state.catalog.installed {
                if subset.is_some_and(|ids| !ids.iter().any(|value| value == id)) {
                    continue;
                }
                if let Ok(plan) = self.plan(&state, "update", id, None) {
                    if plan.version.as_ref().is_some_and(|v| {
                        crate::compare_versions(&v.version, &old.version)
                            == std::cmp::Ordering::Greater
                    }) {
                        plans.push(plan);
                    }
                }
            }
            if plans.is_empty() {
                return Err(CoreError::invalid("没有可更新且兼容的已安装工具"));
            }
        } else {
            plans.push(self.plan(
                &state,
                kind,
                required(params, "toolId")?,
                params["version"].as_str(),
            )?);
        }
        let children = if kind == "updateAll" {
            plans.iter().map(|plan|json!({"toolId":plan.id,"version":plan.version.as_ref().map(|v|v.version.as_str()),"state":"queued","stage":"queued","error":null})).collect()
        } else {
            vec![]
        };
        let settings = state.settings.clone();
        let (id, cancel) = self.create_operation(
            &mut state,
            kind,
            params.clone(),
            if kind == "updateAll" {
                None
            } else {
                Some(plans[0].id.clone())
            },
            plans
                .first()
                .and_then(|p| p.version.as_ref().map(|v| v.version.clone())),
            children,
        )?;
        let service = Arc::clone(self);
        let operation_id = id.clone();
        std::thread::spawn(move || service.run_plans(&operation_id, plans, settings, cancel));
        Ok(json!({"operationId":id}))
    }
    fn change_operation(&self, id: &str, change: impl FnOnce(&mut Operation), persist: bool) {
        let operation = {
            let mut state = self.state.lock().unwrap();
            let Some(operation) = state.operations.get_mut(id) else {
                return;
            };
            change(operation);
            operation.updated_at = now();
            let snapshot = operation.clone();
            self.emit_locked(
                &mut state,
                "operation.changed",
                serde_json::to_value(&snapshot).unwrap_or(Value::Null),
            );
            snapshot
        };
        if persist {
            if let Ok(path) = managed(
                &self.workspace,
                &format!("cache/operations/{id}/operation.json"),
            ) {
                if let Err(error) = atomic_json(&path, &operation) {
                    self.log_error(id, &error);
                }
            }
        }
    }
    fn run_plans(
        self: Arc<Self>,
        id: &str,
        plans: Vec<Plan>,
        settings: Settings,
        cancel: Arc<AtomicBool>,
    ) {
        let _writer = self.writer.lock().unwrap();
        let result = (|| -> CoreResult<Value> {
            network::check_cancel(&cancel)?;
            let lock_path = managed(&self.workspace, "cache/operations/writer.lock")?;
            fs::create_dir_all(lock_path.parent().unwrap())?;
            let writer = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(lock_path)?;
            fs2::FileExt::try_lock_exclusive(&writer)
                .map_err(|_| CoreError::new("BUSY", "其他进程正在修改此工作区"))?;
            self.change_operation(
                id,
                |op| {
                    op.state = "running".into();
                    op.stage = "preflight".into();
                },
                true,
            );
            let batch = plans.len() > 1
                || self
                    .state
                    .lock()
                    .unwrap()
                    .operations
                    .get(id)
                    .is_some_and(|op| op.kind == "updateAll");
            let mut results = vec![];
            for (index, plan) in plans.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    if batch {
                        self.change_operation(
                            id,
                            |op| {
                                for child in op.children.iter_mut().skip(index) {
                                    child["state"] = json!("cancelled");
                                    child["stage"] = json!("cancelled");
                                }
                            },
                            true,
                        );
                    }
                    if results.is_empty() {
                        return Err(CoreError::cancelled());
                    }
                    break;
                }
                if batch {
                    self.change_operation(
                        id,
                        |op| {
                            op.children[index]["state"] = json!("running");
                            op.children[index]["stage"] = json!("preflight");
                        },
                        true,
                    );
                }
                let result = self.execute_plan(id, index, plan, &settings, &cancel);
                if batch {
                    self.change_operation(
                        id,
                        |op| {
                            op.can_cancel = true;
                            let child = &mut op.children[index];
                            child["state"] = json!(match &result {
                                Ok(_) => "succeeded",
                                Err(e) if e.code == "CANCELLED" => "cancelled",
                                Err(e) if e.code == "RECOVERY_REQUIRED" => "recoveryRequired",
                                Err(_) => "failed",
                            });
                            child["stage"] =
                                json!(if result.is_ok() { "complete" } else { "failed" });
                            child["error"] = json!(result.as_ref().err());
                        },
                        true,
                    );
                    results.push(
                        json!({"toolId":plan.id,"ok":result.is_ok(),"error":result.as_ref().err()}),
                    );
                    if let Err(error) = result {
                        self.log_error(id, &error);
                    }
                } else {
                    return result;
                }
            }
            let succeeded = results.iter().filter(|r| r["ok"] == true).count();
            let failed = results.len() - succeeded;
            if succeeded == 0 && failed > 0 {
                return Err(CoreError::new(
                    "BATCH_FAILED",
                    "批量更新失败，请查看各工具结果后重试",
                ));
            }
            Ok(
                json!({"status":if failed>0||cancel.load(Ordering::Relaxed){"partial"}else{"complete"},"succeeded":succeeded,"failed":failed,"results":results}),
            )
        })();
        self.finish_operation(id, result);
    }
    fn execute_plan(
        &self,
        id: &str,
        child: usize,
        plan: &Plan,
        settings: &Settings,
        cancel: &AtomicBool,
    ) -> CoreResult<Value> {
        network::check_cancel(cancel)?;
        let transaction_id = self
            .state
            .lock()
            .unwrap()
            .operations
            .get(id)
            .and_then(|op| op.children.get(child))
            .and_then(|child| child["transactionId"].as_str())
            .unwrap_or(id)
            .to_string();
        self.ensure_not_running(&plan.id)?;
        {
            let state = self.state.lock().unwrap();
            if let Some(error) = &state.catalog.installed_error {
                return Err(CoreError::new("RECOVERY_REQUIRED", error));
            }
            if let Some(version) = &plan.version {
                let current = state
                    .catalog
                    .manifests
                    .get(&plan.id)
                    .ok_or_else(|| CoreError::new("INCOMPATIBLE", "工具清单已不可用"))?;
                current.selected(&version.channel, Some(&version.version), &self.app_version)?;
            }
        }
        if plan.kind == "uninstall" {
            self.enter_commit(id, child, cancel)?;
            transaction::uninstall(&self.workspace, &transaction_id, &plan.id)?;
            self.reload_local();
            return Ok(json!({"toolId":plan.id,"uninstalled":true}));
        }
        let manifest = plan.manifest.as_ref().unwrap();
        let version = plan.version.as_ref().unwrap();
        self.stage(id, child, "download", true);
        let archive = network::download(
            &self.workspace,
            version,
            settings,
            &transaction_id,
            cancel,
            |done, total| {
                self.change_operation(
                    id,
                    |op| {
                        op.done_bytes = done;
                        op.total_bytes = total;
                    },
                    false,
                )
            },
        )?;
        self.stage(id, child, "extract", true);
        let candidate = transaction::prepare(
            &self.workspace,
            &transaction_id,
            manifest,
            version,
            &archive,
            cancel,
        )?;
        network::check_cancel(cancel)?;
        self.ensure_not_running(&plan.id)?;
        let previous = load_installed(&self.workspace)?.0.remove(&plan.id);
        let timestamp = now();
        let record = Installed {
            id: plan.id.clone(),
            name: manifest.name.clone(),
            version: version.version.clone(),
            channel: version.channel.clone(),
            entry: manifest.entry.clone(),
            installed_at: previous
                .as_ref()
                .map(|old| old.installed_at.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| timestamp.clone()),
            updated_at: timestamp,
            extra: previous.map(|old| old.extra).unwrap_or_default(),
        };
        self.enter_commit(id, child, cancel)?;
        transaction::install(&self.workspace, &transaction_id, &candidate, record)?;
        self.reload_local();
        // Only a verified package created under downloads belongs to this operation.
        let _ = fs::remove_file(archive);
        Ok(
            json!({"toolId":plan.id,"version":version.version,"revision":self.state.lock().unwrap().catalog.revision}),
        )
    }
    fn stage(&self, id: &str, child: usize, stage: &str, can_cancel: bool) {
        self.change_operation(
            id,
            |op| {
                op.stage = stage.into();
                op.can_cancel = can_cancel;
                if let Some(item) = op.children.get_mut(child) {
                    item["stage"] = json!(stage);
                }
            },
            true,
        );
    }
    fn enter_commit(&self, id: &str, child: usize, cancel: &AtomicBool) -> CoreResult<()> {
        let operation = {
            let mut state = self.state.lock().unwrap();
            network::check_cancel(cancel)?;
            let op = state
                .operations
                .get_mut(id)
                .ok_or_else(|| CoreError::invalid("任务不存在"))?;
            op.can_cancel = false;
            op.stage = "commit".into();
            if let Some(child) = op.children.get_mut(child) {
                child["stage"] = json!("commit");
            }
            op.updated_at = now();
            let snapshot = op.clone();
            self.emit_locked(
                &mut state,
                "operation.changed",
                serde_json::to_value(&snapshot)?,
            );
            snapshot
        };
        atomic_json(
            &managed(
                &self.workspace,
                &format!("cache/operations/{id}/operation.json"),
            )?,
            &operation,
        )
    }
    fn reload_local(&self) {
        let mut state = self.state.lock().unwrap();
        let mut catalog = Catalog::load(&self.workspace);
        catalog.revision = state.catalog.revision + 1;
        catalog.source = state.catalog.source.clone();
        catalog.recovery = state.catalog.recovery.clone();
        state.catalog = catalog;
        let view = self.catalog_view(&state);
        self.emit_locked(&mut state, "catalog.changed", view);
    }
    fn finish_operation(&self, id: &str, result: CoreResult<Value>) {
        if let Err(error) = &result {
            self.log_error(id, error);
        }
        self.change_operation(
            id,
            |op| {
                op.can_cancel = false;
                match result {
                    Ok(value) => {
                        op.state = "succeeded".into();
                        op.stage = "complete".into();
                        op.result = Some(value);
                    }
                    Err(mut error) => {
                        op.state = match error.code.as_str() {
                            "CANCELLED" => "cancelled",
                            "RECOVERY_REQUIRED" => "recoveryRequired",
                            _ => "failed",
                        }
                        .into();
                        op.stage = if error.code == "CANCELLED" {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        error.operation_id = Some(id.into());
                        op.error = Some(error);
                    }
                }
            },
            true,
        );
        let mut state = self.state.lock().unwrap();
        state.cancellations.remove(id);
        if state
            .operations
            .get(id)
            .is_some_and(|op| op.state == "recoveryRequired")
        {
            if let Some(tool) = state.operations[id].tool_id.clone() {
                state.catalog.recovery.insert(tool);
            }
        }
        let child_recovery: Vec<String> = state
            .operations
            .get(id)
            .into_iter()
            .flat_map(|op| op.children.iter())
            .filter(|child| child["state"] == "recoveryRequired")
            .filter_map(|child| child["toolId"].as_str().map(str::to_string))
            .collect();
        state.catalog.recovery.extend(child_recovery);
        let view = self.catalog_view(&state);
        self.emit_locked(&mut state, "catalog.changed", view);
        drop(state);
        self.prune_operations();
    }
    fn log_error(&self, id: &str, error: &CoreError) {
        if let Ok(path) = managed(&self.workspace, "logs/install-errors.log") {
            if fs::create_dir_all(path.parent().unwrap()).is_ok() {
                if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "[{}] operation={} {}", now(), id, error);
                }
            }
        }
    }
    fn prune_operations(&self) {
        if self.read_only.load(Ordering::Acquire) {
            return;
        }
        let operations = {
            let state = self.state.lock().unwrap();
            if state.catalog.recovery.contains("*") {
                return;
            }
            state.operations.values().cloned().collect::<Vec<_>>()
        };
        let mut eligible: Vec<Operation> = operations
            .into_iter()
            .filter(|operation| {
                if !["succeeded", "failed", "cancelled"].contains(&operation.state.as_str())
                    || operation
                        .children
                        .iter()
                        .any(|child| child["state"] == "recoveryRequired")
                {
                    return false;
                }
                let ids = std::iter::once(operation.id.as_str()).chain(
                    operation
                        .children
                        .iter()
                        .filter_map(|child| child["transactionId"].as_str()),
                );
                !ids.into_iter().any(|id| {
                    let Ok(path) = managed(
                        &self.workspace,
                        &format!("cache/operations/{id}/transaction.json"),
                    ) else {
                        return true;
                    };
                    if !path.exists() {
                        return false;
                    }
                    read_json(&path).map_or(true, |journal| {
                        !matches!(journal["phase"].as_str(), Some("committed" | "rolledBack"))
                    })
                })
            })
            .collect();
        eligible.sort_by_key(|operation| {
            (
                chrono::DateTime::parse_from_rfc3339(&operation.updated_at)
                    .map(|date| date.timestamp_millis())
                    .unwrap_or(i64::MIN),
                operation.id.clone(),
            )
        });
        let remove = eligible.len().saturating_sub(50);
        for operation in eligible.into_iter().take(remove) {
            let Ok(path) = managed(
                &self.workspace,
                &format!("cache/operations/{}/operation.json", operation.id),
            ) else {
                continue;
            };
            // Retention only removes old summaries. Journals, backups and shared diagnostic
            // logs retain their original paths so recovery evidence cannot be severed.
            match fs::remove_file(path) {
                Ok(()) => {
                    self.state.lock().unwrap().operations.remove(&operation.id);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.state.lock().unwrap().operations.remove(&operation.id);
                }
                Err(error) => self.log_error(&operation.id, &error.into()),
            }
        }
    }
    fn ensure_not_running(&self, id: &str) -> CoreResult<()> {
        let mut processes = self.processes.lock().unwrap();
        if let Some(children) = processes.get_mut(id) {
            children.retain_mut(|child| child.try_wait().ok().flatten().is_none());
            if !children.is_empty() {
                return Err(CoreError::new("FILE_IN_USE", "工具仍在运行，请关闭后重试"));
            }
        }
        Ok(())
    }
    fn launch(&self, id: &str) -> CoreResult<()> {
        validate_id(id)?;
        let state = self.state.lock().unwrap();
        if Self::busy_ids(&state, false).contains(id) {
            return Err(CoreError::new("BUSY", "工具正在安装或更新，请等待完成"));
        }
        if state.catalog.recovery.contains(id) {
            return Err(CoreError::new("RECOVERY_REQUIRED", "此工具需要恢复"));
        }
        let record = state
            .catalog
            .installed
            .get(id)
            .ok_or_else(|| CoreError::new("NOT_INSTALLED", "此工具尚未安装"))?;
        let target = managed(&self.workspace, &format!("tools/{id}"))?;
        let entry = managed(&target, &record.entry)?;
        if !entry.is_file() {
            return Err(CoreError::new(
                "MISSING_ENTRY",
                "工具入口不存在，请重新安装或打开工具目录检查",
            ));
        }
        let extension = entry
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let mut command;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if ["cmd", "bat"].contains(&extension.as_str()) {
                let system = std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
                command = Command::new(system.join("System32").join("cmd.exe"));
                command
                    .raw_arg("/D /S /C \"\"%__TOOLBOX_LAUNCH_ENTRY%\"\"")
                    .env("__TOOLBOX_LAUNCH_ENTRY", &entry)
                    .creation_flags(0x08000000);
            } else if extension == "exe" {
                command = Command::new(&entry);
            } else {
                return Err(CoreError::invalid("仅支持登记的 EXE、CMD 与 BAT 入口"));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = extension;
            command = Command::new(&entry);
        }
        command.current_dir(entry.parent().unwrap());
        let child = command.spawn().map_err(|e| {
            CoreError::new(
                "LAUNCH_FAILED",
                format!("启动失败：{e}。请检查工具自身依赖和权限。"),
            )
        })?;
        self.processes
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .push(child);
        Ok(())
    }
    pub fn resolve_action(&self, method: &str, params: &Value) -> CoreResult<NativeAction> {
        match method {
            "system.openWorkspace" => Ok(NativeAction::OpenPath(self.workspace.clone())),
            "system.openLog" => {
                let id = required(params, "operationId")?;
                if !self.state.lock().unwrap().operations.contains_key(id) {
                    return Err(CoreError::invalid("任务不存在"));
                }
                let log = managed(&self.workspace, "logs/install-errors.log")?;
                Ok(NativeAction::OpenPath(if log.exists() {
                    log
                } else {
                    self.workspace.join("logs")
                }))
            }
            "tool.openFolder" => {
                let id = required(params, "toolId")?;
                let state = self.state.lock().unwrap();
                ensure_tool(&state, id)?;
                let path = managed(&self.workspace, &format!("tools/{id}"))?;
                if !path.is_dir() {
                    return Err(CoreError::new("NOT_INSTALLED", "工具目录不存在"));
                }
                Ok(NativeAction::OpenPath(path))
            }
            "tool.copyProjectUrl" => {
                let url = self.project_url(required(params, "toolId")?)?;
                Ok(NativeAction::CopyText(url))
            }
            "about.copyContact" => Ok(NativeAction::CopyText("1548220577".into())),
            "system.openLink" => {
                let target = required(params, "target")?;
                let url = match target {
                    "project" => self.project_url(required(params, "toolId")?)?,
                    "release" => self
                        .state
                        .lock()
                        .unwrap()
                        .catalog
                        .index
                        .toolbox
                        .release_url
                        .clone(),
                    "feedback" => "https://github.com/PhysicalWorldDo/DNFTOOLBOX/issues".into(),
                    "author" => "https://space.bilibili.com/492488982".into(),
                    "homepage" => "https://github.com/PhysicalWorldDo/DNFTOOLBOX".into(),
                    _ => return Err(CoreError::invalid("不支持此链接目标")),
                };
                validate_https(&url)?;
                Ok(NativeAction::OpenUrl(url))
            }
            _ => Err(CoreError::invalid("不支持此原生操作")),
        }
    }
    fn project_url(&self, id: &str) -> CoreResult<String> {
        validate_id(id)?;
        let state = self.state.lock().unwrap();
        let url = state
            .catalog
            .manifests
            .get(id)
            .map(|m| m.project_url.clone())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| CoreError::invalid("此工具没有项目地址"))?;
        validate_https(&url)?;
        Ok(url)
    }
}
fn required<'a>(params: &'a Value, key: &str) -> CoreResult<&'a str> {
    params[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::invalid(format!("缺少参数 {key}")))
}
fn reconcile_interrupted(operation: &mut Operation, workspace: &Path, recovery: &BTreeSet<String>) {
    let committed = |id: &str| {
        validate_id(id).is_ok()
            && managed(
                workspace,
                &format!("cache/operations/{id}/transaction.json"),
            )
            .ok()
            .and_then(|path| read_json(&path).ok())
            .is_some_and(|journal| journal["phase"] == "committed")
    };
    let (succeeded, needs_recovery) = if operation.children.is_empty() {
        (
            usize::from(committed(&operation.id)),
            operation
                .tool_id
                .as_ref()
                .is_some_and(|id| recovery.contains(id)),
        )
    } else {
        for child in &mut operation.children {
            if child["transactionId"].as_str().is_some_and(committed) {
                child["state"] = json!("succeeded");
                child["stage"] = json!("complete");
            } else if child["toolId"]
                .as_str()
                .is_some_and(|id| recovery.contains(id))
            {
                child["state"] = json!("recoveryRequired");
                child["stage"] = json!("interrupted");
            } else if child["state"] == "queued" || child["state"] == "running" {
                child["state"] = json!("cancelled");
                child["stage"] = json!("interrupted");
            }
        }
        (
            operation
                .children
                .iter()
                .filter(|child| child["state"] == "succeeded")
                .count(),
            operation
                .children
                .iter()
                .any(|child| child["state"] == "recoveryRequired"),
        )
    };
    operation.can_cancel = false;
    operation.state = if needs_recovery {
        "recoveryRequired"
    } else if succeeded > 0 {
        "succeeded"
    } else {
        "cancelled"
    }
    .into();
    operation.stage = if succeeded > 0 && !needs_recovery {
        "complete"
    } else {
        "interrupted"
    }
    .into();
    operation.error = if needs_recovery {
        Some(CoreError::new(
            "RECOVERY_REQUIRED",
            "上次运行中断，部分工具需要恢复",
        ))
    } else if succeeded == 0 {
        Some(CoreError::new(
            "INTERRUPTED",
            "上次运行中断，已恢复受管事务",
        ))
    } else {
        None
    };
    if succeeded > 0 {
        operation.result = Some(
            json!({"recovered":true,"status":if operation.children.iter().any(|child|child["state"]!="succeeded"){"partial"}else{"complete"}}),
        );
    }
}
fn ensure_tool(state: &State, id: &str) -> CoreResult<()> {
    validate_id(id)?;
    if state.catalog.manifests.contains_key(id)
        || state.catalog.installed.contains_key(id)
        || state.catalog.index.tools.iter().any(|t| t.id == id)
    {
        Ok(())
    } else {
        Err(CoreError::invalid("工具不存在"))
    }
}
fn validate_https(value: &str) -> CoreResult<()> {
    let parsed = url::Url::parse(value).map_err(|_| CoreError::invalid("链接无效"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(CoreError::invalid("只能打开已声明的 HTTPS 链接"));
    }
    Ok(())
}
fn about() -> Value {
    let logs: Value = serde_json::from_str(include_str!("about-history.json"))
        .expect("embedded release history is valid JSON");
    json!({"title":"物理世界的工具箱","author":"物理世界的欺骗","tagline":"集中安装、更新、启动和管理 DNF 工具的桌面工具箱。","qqNumber":"1548220577","bilibiliUrl":"https://space.bilibili.com/492488982","githubUrl":"https://github.com/PhysicalWorldDo/DNFTOOLBOX","feedbackUrl":"https://github.com/PhysicalWorldDo/DNFTOOLBOX/issues","sponsorQrs":[{"id":"weixin","title":"微信赞助","imageUrl":"assets/sponsor-weixin.jpg"},{"id":"zhifubao","title":"支付宝赞助","imageUrl":"assets/sponsor-zhifubao.jpg"}],"logs":logs})
}
fn directory_warning(workspace: &Path, suppressed: bool) -> Value {
    let name = workspace.to_string_lossy();
    let mut issues = vec![];
    let mut folders = vec![];
    let mut files = vec![];
    let mut names = BTreeSet::new();
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ":._\\/-".contains(c))
    {
        issues.push(json!({"code":"non_english_path","message":"当前路径包含中文、空格或特殊符号，建议改为英文、数字、下划线或短横线。"}));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        if ["Desktop", "Downloads", "Documents"].iter().any(|folder| {
            PathBuf::from(&profile)
                .join(folder)
                .to_string_lossy()
                .eq_ignore_ascii_case(&name)
        }) {
            issues.push(json!({"code":"personal_directory","message":"当前目录像是桌面、下载或文档目录，不建议直接作为工具箱目录。"}));
        }
    }
    if let Ok(entries) = fs::read_dir(workspace) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            names.insert(lower.clone());
            if entry.path().is_dir() {
                if ![
                    "_internal",
                    "cache",
                    "config",
                    "downloads",
                    "logs",
                    "self-update",
                    "tools",
                ]
                .contains(&name.as_str())
                {
                    folders.push(name);
                }
            } else if ![
                "LICENSE",
                "LICENSE.txt",
                "PhysicalWorldToolbox.exe",
                "README.md",
                "README.txt",
                "toolbox.py",
            ]
            .contains(&name.as_str())
                && !lower.ends_with(".log")
                && !(lower.starts_with("physicalworldtoolbox-") && lower.ends_with(".zip"))
            {
                files.push(name);
            }
        }
    }
    if ["dnf.exe", "imagepacks2", "soundpacks", "script.pvf", "tcls"]
        .iter()
        .filter(|marker| names.contains(**marker))
        .count()
        >= 2
    {
        issues.push(json!({"code":"game_directory","message":"当前目录像是游戏目录，不建议与工具箱安装、更新文件混放。"}));
    }
    if folders.len() >= 2 {
        issues.push(json!({"code":"extra_folders","message":"当前目录包含多个非工具箱文件夹，建议把工具箱放到独立目录。"}));
    }
    if files.len() >= 5 {
        issues.push(json!({"code":"extra_files","message":"当前目录包含较多非工具箱文件，建议避免与个人文件混放。"}));
    }
    if folders.len() + files.len() >= 8 {
        issues.push(json!({"code":"extra_entries","message":"当前目录包含较多额外内容，工具箱应放在独立文件夹中。"}));
    }
    folders.sort();
    files.sort();
    let message = issues
        .iter()
        .filter_map(|issue| issue["message"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    json!({"show":!suppressed&&!issues.is_empty(),"message":message,"workspace":name,"issues":issues,"extraFolders":folders,"extraFiles":files})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation(id: &str, state: &str, index: usize) -> Operation {
        let timestamp = format!(
            "2026-11-21T{:02}:{:02}:{:02}Z",
            index / 3600,
            index / 60 % 60,
            index % 60
        );
        Operation {
            id: id.into(),
            kind: "update".into(),
            tool_id: Some("test".into()),
            version: Some("1.0.0".into()),
            state: state.into(),
            stage: "complete".into(),
            done_bytes: 0,
            total_bytes: None,
            can_cancel: state == "queued" || state == "running",
            error: None,
            children: vec![],
            result: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            request: json!({}),
        }
    }
    #[test]
    fn retention_keeps_fifty_recent_terminal_summaries_and_all_active_or_recoverable_material() {
        let test_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
        fs::create_dir_all(&test_root).unwrap();
        let dir = tempfile::tempdir_in(test_root).unwrap();
        let service = Service::open(dir.path(), "0.4.0").unwrap();
        let mut operations: Vec<Operation> = (0..60)
            .map(|i| operation(&format!("op-{i:03}"), "succeeded", i))
            .collect();
        operations.extend([
            operation("op-queued", "queued", 0),
            operation("op-running", "running", 0),
            operation("op-recovery", "recoveryRequired", 0),
            operation("op-material", "failed", 0),
        ]);
        let mut parent = operation("op-parent", "failed", 0);
        parent.children =
            vec![json!({"toolId":"test","state":"failed","transactionId":"op-child"})];
        operations.push(parent);
        for operation in operations {
            atomic_json(
                &managed(
                    dir.path(),
                    &format!("cache/operations/{}/operation.json", operation.id),
                )
                .unwrap(),
                &operation,
            )
            .unwrap();
            service
                .state
                .lock()
                .unwrap()
                .operations
                .insert(operation.id.clone(), operation);
        }
        for id in ["op-material", "op-child"] {
            let root = managed(dir.path(), &format!("cache/operations/{id}")).unwrap();
            atomic_json(
                &root.join("transaction.json"),
                &json!({"phase":"committing"}),
            )
            .unwrap();
            fs::create_dir_all(root.join("backup")).unwrap();
            fs::write(root.join("backup/keep.txt"), "old user data").unwrap();
        }
        fs::create_dir_all(dir.path().join("logs")).unwrap();
        fs::write(
            dir.path().join("logs/install-errors.log"),
            "diagnostic history",
        )
        .unwrap();
        service.prune_operations();
        let state = service.state.lock().unwrap();
        assert_eq!(state.operations.len(), 55);
        assert!(!state.operations.contains_key("op-000"));
        assert!(state.operations.contains_key("op-010"));
        drop(state);
        assert!(!dir
            .path()
            .join("cache/operations/op-000/operation.json")
            .exists());
        for id in [
            "op-queued",
            "op-running",
            "op-recovery",
            "op-material",
            "op-parent",
        ] {
            assert!(
                dir.path()
                    .join(format!("cache/operations/{id}/operation.json"))
                    .exists(),
                "{id}"
            );
        }
        for id in ["op-material", "op-child"] {
            assert_eq!(
                fs::read_to_string(
                    dir.path()
                        .join(format!("cache/operations/{id}/backup/keep.txt"))
                )
                .unwrap(),
                "old user data"
            );
            assert!(dir
                .path()
                .join(format!("cache/operations/{id}/transaction.json"))
                .exists());
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("logs/install-errors.log")).unwrap(),
            "diagnostic history"
        );
        service.prune_operations();
        assert_eq!(service.state.lock().unwrap().operations.len(), 55);
    }
}
