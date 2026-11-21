#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = toolbox_host::desktop::run() {
        if let Some(output) = std::env::args()
            .skip_while(|argument| argument != "--output")
            .nth(1)
        {
            let _ = std::fs::write(
                output,
                serde_json::json!({"ok":false,"error":error}).to_string(),
            );
        } else if !std::env::args().any(|argument| argument == "--apply-update") {
            toolbox_host::native::message(&error, false);
        }
        std::process::exit(1);
    }
}
