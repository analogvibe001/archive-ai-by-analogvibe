// Prevents an additional console window on Windows in release builds. Not
// relevant on macOS, but harmless to keep for parity with the Tauri template.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::Manager;

use archive_ai_core::brand_detector::BrandStore;
use archive_ai_core::execution_engine::ExecutionEngine;
use archive_ai_core::logger::Logger;
use archive_ai_core::template_engine::TemplateStore;

use state::AppState;

/// Copy every *.json file from `from` into `to` (non-recursive), skipping
/// files that already exist at the destination. Used to seed the user's
/// writable app-data copies of templates/brands.json from the read-only
/// bundle resources on first launch.
fn seed_json_dir(from: &Path, to: &Path) {
    let _ = std::fs::create_dir_all(to);
    let Ok(entries) = std::fs::read_dir(from) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Some(name) = path.file_name() {
                let dest = to.join(name);
                if !dest.exists() {
                    let _ = std::fs::copy(&path, &dest);
                }
            }
        }
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            let resolver = handle.path();

            // Bundled, read-only defaults (Resources/templates, Resources/data on macOS).
            let resource_dir = resolver.resource_dir().expect("no se pudo resolver el directorio de recursos");
            let bundled_templates = resource_dir.join("templates");
            let bundled_brands = resource_dir.join("data").join("brands.json");

            // Writable per-user copies. Kept outside the app bundle so edits
            // (new brands, custom hierarchies) survive app updates/re-signing
            // instead of writing into Contents/Resources like the old
            // Electron-less launcher version used to.
            let app_data_dir = resolver.app_data_dir().expect("no se pudo resolver el directorio de datos de la app");
            let templates_dir = app_data_dir.join("templates");
            let brands_path = app_data_dir.join("data").join("brands.json");

            seed_json_dir(&bundled_templates, &templates_dir);
            if !brands_path.exists() {
                if let Some(parent) = brands_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&bundled_brands, &brands_path);
            }

            // Logs and the undo record live under the user's home directory,
            // at the exact same paths the pre-Tauri version used — so anyone
            // upgrading in place keeps their log history and can still undo
            // an organization run they kicked off before updating.
            let home_dir: PathBuf = resolver.home_dir().expect("no se pudo resolver el directorio home");
            let log_dir = home_dir.join(".archive-ai-v2-logs");
            let undo_file = home_dir.join(".archive-ai-v2-undo.json");

            let state = AppState {
                logger: Logger::new(log_dir),
                brands: BrandStore::new(brands_path),
                templates: TemplateStore::new(templates_dir),
                execution: ExecutionEngine::new(undo_file),
            };
            app.manage::<state::Shared>(Arc::new(state));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::quick_stats,
            commands::resolve_path,
            commands::analyze,
            commands::execute,
            commands::undo,
            commands::brands_list,
            commands::brands_add,
            commands::brands_update,
            commands::brands_delete,
            commands::templates_list,
            commands::templates_add,
            commands::templates_update,
            commands::templates_delete,
            commands::open_path,
            commands::reveal_path,
            commands::choose_folder,
            commands::ping,
            commands::logs_recent,
            commands::logs_open,
            commands::logs_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar Archive AI");
}
