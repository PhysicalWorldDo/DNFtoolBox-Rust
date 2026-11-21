use serde_json::{json, Value};
use std::{cmp::Ordering, fs, path::PathBuf};
use tempfile::TempDir;
use toolbox_core::{compare_versions, validate_entry, NativeAction, Service};

fn sandbox() -> TempDir {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
    fs::create_dir_all(&base).unwrap();
    tempfile::tempdir_in(base).unwrap()
}
fn write_json(root: &std::path::Path, path: &str, value: Value) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn manifest() -> Value {
    json!({"schemaVersion":1,"id":"palette","name":"阿拉德调色","category":"图像工具","description":"示例","entry":"bin/run.cmd","latest":{"stable":"1.1.0"},"versions":[
        {"version":"1.0.0","channel":"stable","packageUrl":"file:///missing.zip","sha256":"a".repeat(64)},
        {"version":"1.1.0","channel":"stable","packageUrl":"file:///missing.zip","sha256":"a".repeat(64)}]})
}
#[test]
fn semantic_versions_order_numeric_and_prerelease_components() {
    assert_eq!(compare_versions("1.10.0", "1.2.0"), Ordering::Greater);
    assert_eq!(
        compare_versions("1.0.0-beta.10", "1.0.0-beta.2"),
        Ordering::Greater
    );
    assert_eq!(compare_versions("1.0.0", "1.0.0-rc.1"), Ordering::Greater);
}
#[test]
fn windows_package_paths_reject_escape_devices_and_normalization() {
    for path in [
        "../x",
        "C:/x",
        "bin\\run.cmd",
        "bin/CON.txt",
        "bin/name. ",
        "/root",
        "a//b",
        "a/./b",
        "x:stream",
    ] {
        assert!(validate_entry(path).is_err(), "{path}");
    }
    assert!(validate_entry("中文 目录/bin/run.cmd").is_ok());
}
#[test]
fn offline_catalog_uses_install_record_and_cache_without_writing() {
    let dir = sandbox();
    write_json(
        dir.path(),
        "config/registry-cache/index.json",
        json!({"schemaVersion":1,"tools":[{"id":"palette","name":"阿拉德调色","category":"图像工具","manifestUrl":"tools/palette.json"}],"toolbox":{"latestVersion":"0.2.0"}}),
    );
    write_json(
        dir.path(),
        "config/registry-cache/tools/palette.json",
        manifest(),
    );
    write_json(
        dir.path(),
        "config/installed.json",
        json!({"tools":{"palette":{"id":"palette","name":"旧名","version":"1.0.0","channel":"stable","entry":"bin/run.cmd","installedAt":"2020-01-01T00:00:00+08:00","updatedAt":"2020-01-01T00:00:00+08:00"}}}),
    );
    fs::create_dir_all(dir.path().join("tools/palette/bin")).unwrap();
    fs::write(dir.path().join("tools/palette/bin/run.cmd"), "@exit /b 0").unwrap();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let boot = service.bootstrap();
    assert_eq!(boot["catalog"]["source"], "cached");
    assert_eq!(boot["catalog"]["updateCount"], 1);
    assert_eq!(boot["catalog"]["tools"][0]["installedVersion"], "1.0.0");
    assert_eq!(boot["catalog"]["tools"][0]["entryState"], "ready");
    assert_eq!(boot["catalog"]["tools"][0]["latestVersion"], "1.1.0");
    assert_eq!(boot["toolbox"]["hasUpdate"], false);
    assert_eq!(
        boot["about"]["qqChannelUrl"],
        "https://pd.qq.com/s/g907v9hdc"
    );
    assert!(!dir.path().join("config/app.json").exists());
}

#[test]
fn offline_catalog_recovers_single_version_package_without_a_download_source() {
    for channel in [None, Some("beta")] {
        let dir = sandbox();
        let mut package = json!({
            "schemaVersion": 1,
            "id": "dnf_avatar_studio",
            "name": "薛定谔的化裁间",
            "entry": "bin/run.cmd",
            "version": "2.0.4"
        });
        if let Some(value) = channel {
            package["channel"] = json!(value);
            write_json(dir.path(), "config/app.json", json!({"channel": value}));
        }
        write_json(dir.path(), "tools/dnf_avatar_studio/tool.json", package);
        let bin = dir.path().join("tools/dnf_avatar_studio/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("run.cmd"), "@exit /b 0").unwrap();

        let service = Service::open_read_only(dir.path(), "0.4.1").unwrap();
        let boot = service.bootstrap();
        let tool = &boot["catalog"]["tools"][0];
        assert_eq!(tool["installedVersion"], "2.0.4");
        assert_eq!(tool["entryState"], "ready");
        assert!(tool["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("launch")));
        let version = &tool["availableVersions"][0];
        assert_eq!(version["channel"], channel.unwrap_or("stable"));
        assert_eq!(version["canInstall"], false);
        assert!(!dir.path().join("config/installed.json").exists());
    }
}

#[test]
fn installed_record_can_use_an_offline_descriptor_without_version_information() {
    let dir = sandbox();
    write_json(
        dir.path(),
        "config/installed.json",
        json!({"tools":{"old":{
            "id":"old","name":"工具","entry":"run.cmd","version":"1.0.0"
        }}}),
    );
    write_json(
        dir.path(),
        "tools/old/tool.json",
        json!({
            "id":"old","name":"工具","entry":"run.cmd","category":"图像工具"
        }),
    );
    let service = Service::open_read_only(dir.path(), "0.4.1").unwrap();
    let tool = &service.bootstrap()["catalog"]["tools"][0];
    assert_eq!(tool["category"], "图像工具");
    assert_eq!(tool["installedVersion"], "1.0.0");
    assert!(tool["availableVersions"].as_array().unwrap().is_empty());
}

#[test]
fn about_provides_the_complete_qq_channel_invitation() {
    let dir = sandbox();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let boot = service.bootstrap();
    assert_eq!(
        boot["about"]["qqChannelUrl"],
        "https://pd.qq.com/s/g907v9hdc"
    );
    assert_eq!(
        boot["about"]["qqChannelText"],
        "QQ频道:点击链接加入腾讯频道【物理世界的工具箱】：https://pd.qq.com/s/g907v9hdc"
    );
    assert!(boot["about"]["qqNumber"].is_null());
}

#[test]
fn qq_channel_uses_the_controlled_native_link_target() {
    let dir = sandbox();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let action = service
        .resolve_action(
            "system.openLink",
            &json!({"target":"qq-channel","url":"https://example.invalid"}),
        )
        .unwrap();
    assert!(matches!(action, NativeAction::OpenUrl(url) if url == "https://pd.qq.com/s/g907v9hdc"));
}

#[test]
fn legacy_copy_contact_copies_the_channel_invitation() {
    let dir = sandbox();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let action = service
        .resolve_action("about.copyContact", &json!({}))
        .unwrap();
    assert!(
        matches!(action, NativeAction::CopyText(text) if text == "QQ频道:点击链接加入腾讯频道【物理世界的工具箱】：https://pd.qq.com/s/g907v9hdc")
    );
}

#[test]
fn about_keeps_the_eleven_legacy_release_notes() {
    let dir = sandbox();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    let boot = service.bootstrap();
    let logs = boot["about"]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 14);
    assert_eq!(logs[0]["version"], "v0.4.2");
    assert!(logs.iter().any(|log| log["version"] == "v0.4.1"));
    assert!(logs.iter().any(|log| log["version"] == "v0.3.0"));
    assert!(logs.iter().any(|log| log["version"] == "v0.1.0"));
}

#[test]
fn directory_warning_retains_legacy_mixed_folder_detection() {
    let dir = sandbox();
    fs::create_dir(dir.path().join("personal-one")).unwrap();
    fs::create_dir(dir.path().join("personal-two")).unwrap();
    let service = Service::open(dir.path(), "0.4.0").unwrap();
    assert_eq!(service.bootstrap()["directoryWarning"]["show"], true);
}
