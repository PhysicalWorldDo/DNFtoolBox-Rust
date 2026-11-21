use serde_json::{json, Value};
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;
use toolbox_core::{Request, Service};

fn sandbox() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
    fs::create_dir_all(&base).unwrap();
    tempfile::tempdir_in(base).unwrap()
}

fn write_json(root: &Path, path: &str, value: Value) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn catalog(root: &Path) {
    write_json(
        root,
        "config/registry-cache/index.json",
        json!({"schemaVersion":1,"tools":[
            {"id":"palette","name":"阿拉德调色","manifestUrl":"tools/palette.json"},
            {"id":"rep","name":"REP 播放器","manifestUrl":"tools/rep.json"},
            {"id":"skill","name":"技能染色工坊","manifestUrl":"tools/skill.json"}
        ]}),
    );
}

fn preferences(root: &Path, pins: Value, selected: Value) -> Vec<u8> {
    write_json(
        root,
        "config/launcher-ui.json",
        json!({"schemaVersion":1,"pinnedToolIds":pins,"lastSelectedToolId":selected,"customField":{"keep":7}}),
    );
    fs::read(root.join("config/launcher-ui.json")).unwrap()
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
    assert!(response.ok, "{method}: {:?}", response.error);
    response.result.unwrap()
}

#[test]
fn legacy_unpinned_selection_falls_back_in_pin_order_without_writing() {
    let dir = sandbox();
    catalog(dir.path());
    let pins = json!(["missing", "skill", "rep"]);
    let original = preferences(dir.path(), pins.clone(), json!("palette"));

    for read_only in [false, true] {
        let service = if read_only {
            Service::open_read_only(dir.path(), "0.4.0")
        } else {
            Service::open(dir.path(), "0.4.0")
        }
        .unwrap();
        let loaded = &service.bootstrap()["preferences"];
        assert_eq!(loaded["lastSelectedToolId"], "skill");
        assert_eq!(loaded["pinnedToolIds"], pins);
        assert_eq!(
            fs::read(dir.path().join("config/launcher-ui.json")).unwrap(),
            original
        );
    }
}

#[test]
fn indexed_pinned_selection_survives_a_missing_manifest() {
    let dir = sandbox();
    catalog(dir.path());
    preferences(dir.path(), json!(["rep", "skill"]), json!("skill"));
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    assert_eq!(
        service.bootstrap()["preferences"]["lastSelectedToolId"],
        "skill"
    );
}

#[test]
fn no_available_pins_clear_selection_without_removing_saved_ids() {
    for pins in [json!([]), json!(["missing"])] {
        let dir = sandbox();
        catalog(dir.path());
        let original = preferences(dir.path(), pins.clone(), json!("palette"));
        let service = Service::open_read_only(dir.path(), "0.4.0").unwrap();
        let loaded = &service.bootstrap()["preferences"];
        assert!(loaded["lastSelectedToolId"].is_null());
        assert_eq!(loaded["pinnedToolIds"], pins);
        assert_eq!(
            fs::read(dir.path().join("config/launcher-ui.json")).unwrap(),
            original
        );
    }
}

#[test]
fn unavailable_catalog_preserves_preferences_for_a_later_successful_load() {
    let dir = sandbox();
    let pins = json!(["missing", "rep", "skill"]);
    let original = preferences(dir.path(), pins.clone(), json!("skill"));
    write_json(
        dir.path(),
        "config/registry-cache/index.json",
        json!({"schemaVersion":99,"tools":[]}),
    );
    let service = Service::open_read_only(dir.path(), "0.4.0").unwrap();
    assert!(service.bootstrap()["preferences"]["lastSelectedToolId"].is_null());
    assert_eq!(service.bootstrap()["preferences"]["pinnedToolIds"], pins);
    assert_eq!(
        fs::read(dir.path().join("config/launcher-ui.json")).unwrap(),
        original
    );
    catalog(dir.path());
    let restored = Service::open_read_only(dir.path(), "0.4.0").unwrap();
    assert_eq!(
        restored.bootstrap()["preferences"]["lastSelectedToolId"],
        "skill"
    );
}

#[tokio::test]
async fn selecting_an_unpinned_tool_keeps_the_current_pinned_selection() {
    let dir = sandbox();
    catalog(dir.path());
    preferences(dir.path(), json!(["rep", "skill"]), json!("rep"));
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    call(&service, "preferences.selectTool", json!({"toolId":"rep"})).await;
    let saved = call(
        &service,
        "preferences.selectTool",
        json!({"toolId":"palette"}),
    )
    .await;
    assert_eq!(saved["lastSelectedToolId"], "rep");
    assert_eq!(saved["pinnedToolIds"], json!(["rep", "skill"]));
    assert_eq!(saved["customField"]["keep"], 7);
    let selected = call(
        &service,
        "preferences.selectTool",
        json!({"toolId":"skill"}),
    )
    .await;
    assert_eq!(selected["lastSelectedToolId"], "skill");
}

#[tokio::test]
async fn unpinning_the_selected_tool_uses_the_next_available_pin_then_clears() {
    let dir = sandbox();
    catalog(dir.path());
    preferences(
        dir.path(),
        json!(["missing", "skill", "rep"]),
        json!("skill"),
    );
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    call(
        &service,
        "preferences.selectTool",
        json!({"toolId":"skill"}),
    )
    .await;
    let mut events = service.subscribe();
    let saved = call(
        &service,
        "preferences.setPinned",
        json!({"toolId":"skill","pinned":false}),
    )
    .await;
    assert_eq!(saved["lastSelectedToolId"], "rep");
    assert_eq!(saved["pinnedToolIds"], json!(["missing", "rep"]));
    let event = events.try_recv().unwrap();
    assert_eq!(event.event, "preferences.changed");
    assert_eq!(event.data, saved);
    let empty = call(
        &service,
        "preferences.setPinned",
        json!({"toolId":"rep","pinned":false}),
    )
    .await;
    assert!(empty["lastSelectedToolId"].is_null());
    assert_eq!(empty["pinnedToolIds"], json!(["missing"]));
    let disk: Value =
        serde_json::from_slice(&fs::read(dir.path().join("config/launcher-ui.json")).unwrap())
            .unwrap();
    assert!(disk["lastSelectedToolId"].is_null());
    assert_eq!(disk["pinnedToolIds"], json!(["missing"]));
    assert_eq!(disk["customField"]["keep"], 7);
    assert!(disk.get("revision").is_none());
}

#[tokio::test]
async fn pinning_an_already_pinned_tool_keeps_its_position() {
    let dir = sandbox();
    catalog(dir.path());
    preferences(dir.path(), json!(["rep", "skill"]), json!("skill"));
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let saved = call(
        &service,
        "preferences.setPinned",
        json!({"toolId":"rep","pinned":true}),
    )
    .await;
    assert_eq!(saved["pinnedToolIds"], json!(["rep", "skill"]));
    assert_eq!(saved["lastSelectedToolId"], "skill");
}

#[tokio::test]
async fn pinning_the_first_tool_selects_it_without_removing_unavailable_pins() {
    let dir = sandbox();
    catalog(dir.path());
    preferences(dir.path(), json!(["missing"]), Value::Null);
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let saved = call(
        &service,
        "preferences.setPinned",
        json!({"toolId":"rep","pinned":true}),
    )
    .await;
    assert_eq!(saved["lastSelectedToolId"], "rep");
    assert_eq!(saved["pinnedToolIds"], json!(["missing", "rep"]));
}
