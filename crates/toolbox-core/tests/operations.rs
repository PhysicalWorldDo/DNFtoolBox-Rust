use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use toolbox_core::{Request, Service};
use zip::{write::SimpleFileOptions, ZipWriter};

fn sandbox() -> TempDir {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
    fs::create_dir_all(&base).unwrap();
    tempfile::tempdir_in(base).unwrap()
}
fn write_json(root: &Path, path: &str, value: Value) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn package(root: &Path, version: &str, files: &[(&str, &str)]) -> (String, String) {
    package_for(root, "palette", version, files)
}
fn package_for(root: &Path, id: &str, version: &str, files: &[(&str, &str)]) -> (String, String) {
    let path = root.join(format!("{id}-{version}.zip"));
    let mut writer = ZipWriter::new(fs::File::create(&path).unwrap());
    writer
        .start_file("tool.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(serde_json::to_string(&json!({"schemaVersion":1,"id":id,"name":"调色","entry":"bin/run.cmd","latest":{"stable":version},"versions":[{"version":version,"channel":"stable"}]})).unwrap().as_bytes()).unwrap();
    for (name, data) in files {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    let hash = hex::encode(Sha256::digest(fs::read(&path).unwrap()));
    (url::Url::from_file_path(path).unwrap().to_string(), hash)
}
fn setup(root: &Path, versions: Vec<Value>) {
    write_json(
        root,
        "config/app.json",
        json!({"indexUrl":root.join("repo/index.json").to_string_lossy(),"channel":"stable","githubProxy":{"enabled":false,"urls":[]},"unknownSetting":{"keep":7}}),
    );
    write_json(
        root,
        "repo/index.json",
        json!({"schemaVersion":1,"tools":[{"id":"palette","name":"调色","category":"图像工具","manifestUrl":"palette.json"}]}),
    );
    let latest = versions.last().unwrap()["version"].clone();
    write_json(
        root,
        "repo/palette.json",
        json!({"schemaVersion":1,"id":"palette","name":"调色","category":"图像工具","entry":"bin/run.cmd","latest":{"stable":latest},"versions":versions}),
    );
}
async fn call(service: &Arc<Service>, method: &str, params: Value) -> Value {
    let response = service
        .dispatch(Request {
            v: 1,
            id: uuid::Uuid::new_v4().to_string(),
            method: method.into(),
            params,
        })
        .await;
    assert!(response.ok, "{}: {:?}", method, response.error);
    response.result.unwrap()
}
async fn finished(service: &Arc<Service>, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let operations = call(service, "operation.list", json!({})).await;
        let op = operations
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap();
        if !["queued", "running"].contains(&op["state"].as_str().unwrap()) {
            return op.clone();
        }
        assert!(Instant::now() < deadline, "task timed out: {op}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
async fn refresh(service: &Arc<Service>) {
    let op = call(service, "catalog.refresh", json!({"reason":"manual"})).await;
    let result = finished(service, op["operationId"].as_str().unwrap()).await;
    assert_eq!(result["state"], "succeeded", "{result}");
}
#[tokio::test]
async fn settings_patch_preserves_unknown_fields_and_reports_revision_conflicts() {
    let dir = sandbox();
    write_json(
        dir.path(),
        "config/app.json",
        json!({"unknownSetting":{"keep":7}}),
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let original = service.bootstrap()["settings"].clone();
    let saved = call(&service,"settings.update",json!({"revision":original["revision"],"patch":{"githubProxy":{"enabled":false,"urls":[" example.com ","https://example.com/"]}}})).await;
    assert_eq!(
        saved["githubProxy"]["urls"],
        json!(["https://example.com/"])
    );
    let disk: Value =
        serde_json::from_slice(&fs::read(dir.path().join("config/app.json")).unwrap()).unwrap();
    assert_eq!(disk["unknownSetting"]["keep"], 7);
    let stale = service
        .dispatch(Request {
            v: 1,
            id: "stale".into(),
            method: "settings.update".into(),
            params: json!({"revision":1,"patch":{"channel":"beta"}}),
        })
        .await;
    assert_eq!(stale.error.unwrap().code, "STALE_REVISION");
}
#[tokio::test]
async fn install_switch_and_uninstall_preserve_user_data_and_commit_records() {
    let dir = sandbox();
    let (url1, hash1) = package(
        dir.path(),
        "1.0.0",
        &[
            ("bin/run.cmd", "@exit /b 0"),
            ("bin/obsolete.txt", "old"),
            ("config/user.txt", "default"),
            ("data/value.txt", "default"),
        ],
    );
    let (url2, hash2) = package(
        dir.path(),
        "1.1.0",
        &[
            ("bin/run.cmd", "@exit /b 1"),
            ("config/user.txt", "new-default"),
            ("config/added.txt", "added"),
            ("data/value.txt", "new-default"),
        ],
    );
    setup(
        dir.path(),
        vec![
            json!({"version":"1.0.0","packageUrl":url1,"sha256":hash1}),
            json!({"version":"1.1.0","packageUrl":url2,"sha256":hash2}),
        ],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let install = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette","version":"1.0.0"}),
    )
    .await;
    let task = finished(&service, install["operationId"].as_str().unwrap()).await;
    assert_eq!(task["state"], "succeeded", "{task}");
    fs::write(
        dir.path().join("tools/palette/config/user.txt"),
        "my settings",
    )
    .unwrap();
    fs::write(dir.path().join("tools/palette/data/value.txt"), "my data").unwrap();
    fs::write(dir.path().join("tools/palette/kept.txt"), "custom").unwrap();
    let first: Value =
        serde_json::from_slice(&fs::read(dir.path().join("config/installed.json")).unwrap())
            .unwrap();
    let update = call(
        &service,
        "operation.start",
        json!({"kind":"update","toolId":"palette"}),
    )
    .await;
    let task = finished(&service, update["operationId"].as_str().unwrap()).await;
    assert_eq!(task["state"], "succeeded", "{task}");
    assert_eq!(
        fs::read_to_string(dir.path().join("tools/palette/config/user.txt")).unwrap(),
        "my settings"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tools/palette/data/value.txt")).unwrap(),
        "my data"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tools/palette/kept.txt")).unwrap(),
        "custom"
    );
    assert!(!dir.path().join("tools/palette/bin/obsolete.txt").exists());
    assert!(dir.path().join("tools/palette/config/added.txt").exists());
    let second: Value =
        serde_json::from_slice(&fs::read(dir.path().join("config/installed.json")).unwrap())
            .unwrap();
    assert_eq!(
        first["tools"]["palette"]["installedAt"],
        second["tools"]["palette"]["installedAt"]
    );
    assert_eq!(second["tools"]["palette"]["version"], "1.1.0");
    let switch = call(
        &service,
        "operation.start",
        json!({"kind":"switchVersion","toolId":"palette","version":"1.0.0"}),
    )
    .await;
    assert_eq!(
        finished(&service, switch["operationId"].as_str().unwrap()).await["state"],
        "succeeded"
    );
    let unconfirmed = service
        .dispatch(Request {
            v: 1,
            id: "unconfirmed".into(),
            method: "operation.start".into(),
            params: json!({"kind":"uninstall","toolId":"palette"}),
        })
        .await;
    assert!(!unconfirmed.ok);
    assert!(dir.path().join("tools/palette/config/user.txt").exists());
    let uninstall = call(
        &service,
        "operation.start",
        json!({"kind":"uninstall","toolId":"palette","confirmed":true}),
    )
    .await;
    assert_eq!(
        finished(&service, uninstall["operationId"].as_str().unwrap()).await["state"],
        "succeeded"
    );
    assert!(!dir.path().join("tools/palette").exists());
    assert_eq!(
        service.bootstrap()["catalog"]["tools"][0]["installed"],
        false
    );
}
#[tokio::test]
async fn installs_zip_members_with_windows_separators() {
    let dir = sandbox();
    let (url, hash) = package(
        dir.path(),
        "1.0.0",
        &[
            (r"bin\run.cmd", "entry"),
            (r"bin\app\readme.txt", "contents"),
        ],
    );
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":url,"sha256":hash})],
    );
    let service = Service::open(dir.path(), "0.4.2").unwrap();
    refresh(&service).await;
    let start = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette"}),
    )
    .await;
    let result = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(result["state"], "succeeded", "{result}");
    let target = dir.path().join("tools/palette");
    assert_eq!(
        fs::read_to_string(target.join("bin/run.cmd")).unwrap(),
        "entry"
    );
    assert_eq!(
        fs::read_to_string(target.join("bin/app/readme.txt")).unwrap(),
        "contents"
    );
}

#[tokio::test]
async fn rejects_zip_members_that_normalize_to_the_same_path() {
    let dir = sandbox();
    let (url, hash) = package(
        dir.path(),
        "1.0.0",
        &[("bin/run.cmd", "first"), (r"BIN\run.cmd", "second")],
    );
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":url,"sha256":hash})],
    );
    let service = Service::open(dir.path(), "0.4.2").unwrap();
    refresh(&service).await;
    let start = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette"}),
    )
    .await;
    let result = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(result["state"], "failed", "{result}");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("重复或大小写冲突"),
        "{result}"
    );
    assert!(!dir.path().join("tools/palette").exists());
    assert!(!dir.path().join("config/installed.json").exists());
}

#[tokio::test]
async fn bad_hash_and_zip_escape_never_create_install_records() {
    let dir = sandbox();
    let (url, _) = package(dir.path(), "1.0.0", &[("bin/run.cmd", "@exit /b 0")]);
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":url,"sha256":"f".repeat(64)})],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let start = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette"}),
    )
    .await;
    let result = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(result["state"], "failed");
    assert_eq!(result["error"]["code"], "HASH_MISMATCH");
    assert!(!dir.path().join("config/installed.json").exists());
    let (url, hash) = package(
        dir.path(),
        "1.0.0",
        &[("../escape.txt", "escape"), ("bin/run.cmd", "cmd")],
    );
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":url,"sha256":hash})],
    );
    refresh(&service).await;
    let start = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette"}),
    )
    .await;
    let result = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(result["state"], "failed");
    assert!(!dir.path().join("tools/palette").exists());
    assert!(!dir.path().join("cache/installing/escape.txt").exists());
}
#[tokio::test]
async fn failed_refresh_retains_previous_complete_catalog_and_deduplicates_requests() {
    let dir = sandbox();
    let (url, hash) = package(dir.path(), "1.0.0", &[("bin/run.cmd", "cmd")]);
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":url,"sha256":hash})],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let before = service.bootstrap()["catalog"]["revision"].clone();
    fs::remove_file(dir.path().join("repo/palette.json")).unwrap();
    let request = Request {
        v: 1,
        id: "dedupe".into(),
        method: "catalog.refresh".into(),
        params: json!({"reason":"manual"}),
    };
    let first = service.dispatch(request.clone()).await;
    let second = service.dispatch(request).await;
    assert_eq!(first.result, second.result);
    let result = finished(
        &service,
        first.result.unwrap()["operationId"].as_str().unwrap(),
    )
    .await;
    assert_eq!(result["state"], "failed");
    assert_eq!(service.bootstrap()["catalog"]["revision"], before);
    assert_eq!(
        service.bootstrap()["catalog"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let reopened = Service::open(dir.path(), "0.4.0").unwrap();
    assert_eq!(
        reopened.bootstrap()["catalog"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[cfg(windows)]
#[tokio::test]
async fn launches_real_cmd_in_chinese_space_and_percent_path_with_entry_parent_cwd() {
    let dir = sandbox();
    let workspace = dir.path().join("中文 空格 & 100%");
    fs::create_dir_all(workspace.join("tools/palette/bin")).unwrap();
    write_json(
        &workspace,
        "config/installed.json",
        json!({"tools":{"palette":{"id":"palette","name":"调色","version":"1.0.0","channel":"stable","entry":"bin/run.cmd","installedAt":"original","updatedAt":"original"}}}),
    );
    fs::write(
        workspace.join("tools/palette/bin/run.cmd"),
        "@echo off\r\necho launched> \"%~dp0launched.txt\"\r\ncd> \"%~dp0cwd.txt\"\r\n",
    )
    .unwrap();
    let service = Service::open(&workspace, "0.4.0").unwrap();
    call(&service, "tool.launch", json!({"toolId":"palette"})).await;
    let deadline = Instant::now() + Duration::from_secs(4);
    let marker = workspace.join("tools/palette/bin/launched.txt");
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.exists(), "CMD never ran: {}", workspace.display());
    let cwd_file = workspace.join("tools/palette/bin/cwd.txt");
    while !cwd_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(cwd_file.exists());
    let cwd = String::from_utf8_lossy(&fs::read(cwd_file).unwrap()).into_owned();
    assert!(cwd.contains("palette\\bin"), "{cwd}");
}

#[tokio::test]
async fn cancelling_streamed_download_removes_partial_and_never_changes_installs() {
    use std::{io::Read, net::TcpListener};
    let dir = sandbox();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let data = vec![42_u8; 4 * 1024 * 1024];
    let hash = hex::encode(Sha256::digest(&data));
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request);
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        for chunk in data.chunks(65536) {
            if socket.write_all(chunk).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    setup(
        dir.path(),
        vec![
            json!({"version":"1.0.0","packageUrl":format!("http://127.0.0.1:{port}/package.zip"),"sha256":hash}),
        ],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let start = call(
        &service,
        "operation.start",
        json!({"kind":"install","toolId":"palette"}),
    )
    .await;
    let id = start["operationId"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let ops = call(&service, "operation.list", json!({})).await;
        let op = ops
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["id"] == id)
            .unwrap();
        if op["doneBytes"].as_u64().unwrap() > 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        call(&service, "operation.cancel", json!({"operationId":id})).await["accepted"],
        true
    );
    assert_eq!(finished(&service, id).await["state"], "cancelled");
    server.join().unwrap();
    assert!(!dir.path().join("config/installed.json").exists());
    assert!(!dir.path().join("tools/palette").exists());
    assert!(!fs::read_dir(dir.path().join("downloads"))
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "part")));
}

#[tokio::test]
async fn batch_updates_continue_after_failure_and_retry_only_failed_installed_tools() {
    let dir = sandbox();
    let root = dir.path();
    let (first_url, first_hash) =
        package_for(root, "palette", "1.1.0", &[("bin/run.cmd", "@exit /b 0")]);
    let (second_url, second_hash) =
        package_for(root, "second", "1.1.0", &[("bin/run.cmd", "@exit /b 0")]);
    let second_path = url::Url::parse(&second_url)
        .unwrap()
        .to_file_path()
        .unwrap();
    let good_bytes = fs::read(&second_path).unwrap();
    fs::write(&second_path, b"bad archive").unwrap();
    write_json(
        root,
        "config/app.json",
        json!({"indexUrl":root.join("repo/index.json").to_string_lossy(),"channel":"stable","githubProxy":{"enabled":false,"urls":[]}}),
    );
    let mut tools = vec![];
    let mut installed = serde_json::Map::new();
    for (id, address, hash, install) in [
        ("palette", first_url.clone(), first_hash.clone(), true),
        ("second", second_url, second_hash, true),
        ("newtool", first_url, first_hash, false),
    ] {
        tools.push(json!({"id":id,"name":id,"manifestUrl":format!("{id}.json")}));
        write_json(
            root,
            &format!("repo/{id}.json"),
            json!({"schemaVersion":1,"id":id,"name":id,"entry":"bin/run.cmd","latest":{"stable":"1.1.0"},"versions":[{"version":"1.1.0","packageUrl":address,"sha256":hash}]}),
        );
        if install {
            fs::create_dir_all(root.join(format!("tools/{id}/bin"))).unwrap();
            fs::write(root.join(format!("tools/{id}/bin/run.cmd")), "old").unwrap();
            installed.insert(id.into(),json!({"id":id,"name":id,"version":"1.0.0","channel":"stable","entry":"bin/run.cmd","installedAt":"original","updatedAt":"old"}));
        }
    }
    write_json(
        root,
        "repo/index.json",
        json!({"schemaVersion":1,"tools":tools}),
    );
    write_json(root, "config/installed.json", json!({"tools":installed}));
    let service = Service::open(root, "0.4.0").unwrap();
    refresh(&service).await;
    let start = call(&service, "operation.start", json!({"kind":"updateAll"})).await;
    let id = start["operationId"].as_str().unwrap();
    let result = finished(&service, id).await;
    assert_eq!(result["state"], "succeeded", "{result}");
    assert_eq!(result["result"]["status"], "partial");
    assert_eq!(result["children"].as_array().unwrap().len(), 2);
    assert!(!root.join("tools/newtool").exists());
    assert_eq!(result["children"][0]["state"], "succeeded");
    assert_eq!(result["children"][1]["state"], "failed");
    fs::write(second_path, good_bytes).unwrap();
    let retry = call(&service, "operation.retry", json!({"operationId":id})).await;
    let retried = finished(&service, retry["operationId"].as_str().unwrap()).await;
    assert_eq!(retried["state"], "succeeded", "{retried}");
    assert_eq!(retried["children"].as_array().unwrap().len(), 1);
    assert_eq!(retried["children"][0]["toolId"], "second");
}

#[tokio::test]
async fn trial_mode_is_read_only_until_health_commit_enables_writes() {
    let dir = sandbox();
    let service = Service::open_read_only(dir.path(), "0.4.0").unwrap();
    let denied = service
        .dispatch(Request {
            v: 1,
            id: "denied".into(),
            method: "settings.update".into(),
            params: json!({"revision":1,"patch":{"channel":"beta"}}),
        })
        .await;
    assert_eq!(denied.error.unwrap().code, "BUSY");
    assert!(!dir.path().join("config").exists());
    service.enable_writes_after_trial().unwrap();
    assert_eq!(service.bootstrap()["capabilities"]["install"], true);
    assert_eq!(
        call(
            &service,
            "settings.update",
            json!({"revision":1,"patch":{"channel":"beta"}})
        )
        .await["channel"],
        "beta"
    );
}

#[tokio::test]
async fn failed_package_identity_in_batch_does_not_contaminate_next_tools_staging() {
    let dir = sandbox();
    let root = dir.path();
    let (wrong_url, wrong_hash) = package_for(root, "wrong", "1.1.0", &[("bin/run.cmd", "new")]);
    let (good_url, good_hash) = package_for(root, "second", "1.1.0", &[("bin/run.cmd", "new")]);
    write_json(
        root,
        "config/app.json",
        json!({"indexUrl":root.join("repo/index.json").to_string_lossy(),"githubProxy":{"enabled":false,"urls":[]}}),
    );
    let mut tools = vec![];
    let mut installed = serde_json::Map::new();
    for (id, address, hash) in [
        ("palette", wrong_url, wrong_hash),
        ("second", good_url, good_hash),
    ] {
        tools.push(json!({"id":id,"name":id,"manifestUrl":format!("{id}.json")}));
        write_json(
            root,
            &format!("repo/{id}.json"),
            json!({"schemaVersion":1,"id":id,"name":id,"entry":"bin/run.cmd","latest":{"stable":"1.1.0"},"versions":[{"version":"1.1.0","packageUrl":address,"sha256":hash}]}),
        );
        fs::create_dir_all(root.join(format!("tools/{id}/bin"))).unwrap();
        fs::write(root.join(format!("tools/{id}/bin/run.cmd")), "old").unwrap();
        installed.insert(id.into(),json!({"id":id,"name":id,"version":"1.0.0","entry":"bin/run.cmd","installedAt":"original","updatedAt":"old"}));
    }
    write_json(
        root,
        "repo/index.json",
        json!({"schemaVersion":1,"tools":tools}),
    );
    write_json(root, "config/installed.json", json!({"tools":installed}));
    let service = Service::open(root, "0.4.0").unwrap();
    refresh(&service).await;
    let start = call(&service, "operation.start", json!({"kind":"updateAll"})).await;
    let result = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(result["children"][0]["state"], "failed");
    assert_eq!(result["children"][1]["state"], "succeeded", "{result}");
    assert_eq!(
        fs::read_to_string(root.join("tools/palette/bin/run.cmd")).unwrap(),
        "old"
    );
}

#[tokio::test]
async fn registry_history_without_package_is_displayed_but_cannot_be_installed() {
    let dir = sandbox();
    let (address, hash) = package(dir.path(), "2.2.0", &[("bin/run.cmd", "cmd")]);
    setup(
        dir.path(),
        vec![
            json!({"version":"0.4.5","channel":"stable","packageUrl":"","sha256":"A".repeat(64),"changelog":["历史发行"]}),
            json!({"version":"2.2.0","channel":"stable","packageUrl":address,"sha256":hash}),
        ],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let tool = call(&service, "catalog.tool", json!({"toolId":"palette"})).await;
    assert_eq!(tool["availableVersions"].as_array().unwrap().len(), 2);
    let historical = tool["availableVersions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["version"] == "0.4.5")
        .unwrap();
    assert_eq!(historical["canInstall"], false);
    assert!(historical["disabledReason"]
        .as_str()
        .unwrap()
        .contains("安装包"));
    assert!(tool["actions"]
        .as_array()
        .unwrap()
        .contains(&json!("install")));
    let denied = service
        .dispatch(Request {
            v: 1,
            id: "old-version".into(),
            method: "operation.start".into(),
            params: json!({"kind":"install","toolId":"palette","version":"0.4.5"}),
        })
        .await;
    assert_eq!(denied.error.unwrap().code, "INCOMPATIBLE");
    assert!(!dir.path().join("config/installed.json").exists());
}

#[tokio::test]
async fn refresh_rejected_after_source_change_does_not_publish_a_stale_cache_generation() {
    use std::{io::Read, net::TcpListener, sync::mpsc};
    let dir = sandbox();
    let (address, hash) = package(dir.path(), "1.0.0", &[("bin/run.cmd", "cmd")]);
    setup(
        dir.path(),
        vec![json!({"version":"1.0.0","packageUrl":address,"sha256":hash})],
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request).unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let body = r#"{"schemaVersion":1,"tools":[]}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    call(
        &service,
        "settings.update",
        json!({"revision":1,"patch":{"indexUrl":format!("http://127.0.0.1:{port}/index.json")}}),
    )
    .await;
    let start = call(&service, "catalog.refresh", json!({"reason":"manual"})).await;
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    call(&service,"settings.update",json!({"revision":2,"patch":{"indexUrl":dir.path().join("repo/index.json").to_string_lossy()}})).await;
    release_tx.send(()).unwrap();
    let task = finished(&service, start["operationId"].as_str().unwrap()).await;
    assert_eq!(task["error"]["code"], "STALE_REVISION");
    server.join().unwrap();
    let reopened = Service::open(dir.path(), "0.4.0").unwrap();
    assert_eq!(
        reopened.bootstrap()["catalog"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn compatible_installed_release_can_be_repaired_when_latest_requires_newer_toolbox() {
    let dir = sandbox();
    let (address, hash) = package(dir.path(), "1.0.0", &[("bin/run.cmd", "cmd")]);
    setup(
        dir.path(),
        vec![
            json!({"version":"1.0.0","packageUrl":address,"sha256":hash}),
            json!({"version":"2.0.0","packageUrl":"file:///not-needed.zip","sha256":"A".repeat(64),"minToolboxVersion":"99.0.0"}),
        ],
    );
    write_json(
        dir.path(),
        "config/installed.json",
        json!({"tools":{"palette":{"id":"palette","name":"调色","version":"1.0.0","entry":"bin/run.cmd","installedAt":"original","updatedAt":"old"}}}),
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    refresh(&service).await;
    let tool = call(&service, "catalog.tool", json!({"toolId":"palette"})).await;
    assert_eq!(tool["entryState"], "missing");
    assert!(tool["actions"]
        .as_array()
        .unwrap()
        .contains(&json!("reinstall")));
    assert!(!tool["actions"]
        .as_array()
        .unwrap()
        .contains(&json!("update")));
}
