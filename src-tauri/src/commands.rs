use serde::Serialize;
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Emitter, State};

use archive_ai_core::brand_detector::{Brand, BrandInput, BrandPatch};
use archive_ai_core::execution_engine::{ApplyResult, UndoResult};
use archive_ai_core::project_analyzer::{self, Proposal};
use archive_ai_core::scanner::{self, Stats};
use archive_ai_core::smb_resolver::{self, ResolvedPath};
use archive_ai_core::template_engine::Template;

use crate::state::Shared;

fn to_str_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ── Paths / stats ─────────────────────────────────────────────────────
#[tauri::command]
pub async fn quick_stats(path: String, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let resolved = smb_resolver::resolve_path(&path, &shared.logger)?;
        let folder = Path::new(&resolved.resolved_path);
        let stats = scanner::quick_stats(folder, &shared.logger)?;
        let mut value = serde_json::to_value(&stats).map_err(to_str_err)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert("resolvedPath".into(), serde_json::Value::String(resolved.resolved_path));
            map.insert("autoCorrected".into(), serde_json::Value::Bool(resolved.auto_corrected.unwrap_or(false)));
        }
        Ok(value)
    })
    .await
    .map_err(to_str_err)?
}

#[tauri::command]
pub async fn resolve_path(path: String, state: State<'_, Shared>) -> Result<ResolvedPath, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || smb_resolver::resolve_path(&path, &shared.logger))
        .await
        .map_err(to_str_err)?
}

// ── Analyze (emits "analyze-progress" events, then resolves with the result) ──
#[derive(Serialize, Clone)]
struct ProgressPayload {
    step: String,
    pct: u32,
    #[serde(skip_serializing_if = "Option::is_none", rename = "resolvedPath")]
    resolved_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "folderCount")]
    folder_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "fileCount")]
    file_count: Option<u64>,
}

#[derive(Serialize)]
struct TreeChildSummary {
    name: String,
    path: String,
    stats: Stats,
    #[serde(rename = "childCount")]
    child_count: usize,
}

#[derive(Serialize)]
struct TreeSummary {
    name: String,
    children: Vec<TreeChildSummary>,
    stats: Stats,
}

#[tauri::command]
pub async fn analyze(
    path: String,
    template_id: Option<String>,
    app: AppHandle,
    state: State<'_, Shared>,
) -> Result<serde_json::Value, String> {
    let shared = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let emit = |app: &AppHandle, step: &str, pct: u32, resolved_path: Option<String>, folder_count: Option<i64>, file_count: Option<u64>| {
            let _ = app.emit(
                "analyze-progress",
                ProgressPayload { step: step.to_string(), pct, resolved_path, folder_count, file_count },
            );
        };

        if smb_resolver::is_smb_url(&path) {
            emit(&app, "Conectando al servidor de red...", 5, None, None, None);
        }
        let resolved = smb_resolver::resolve_path(&path, &shared.logger)?;
        let folder_path = resolved.resolved_path.clone();
        if resolved.auto_corrected.unwrap_or(false) {
            emit(&app, "Ruta corregida automáticamente...", 8, Some(folder_path.clone()), None, None);
        }

        emit(&app, "Escaneando estructura de carpetas...", 10, None, None, None);
        let tree = scanner::scan(Path::new(&folder_path), &shared.logger)?;
        emit(
            &app,
            "Detectando marcas y fechas...",
            40,
            None,
            Some(tree.children.len() as i64),
            Some(tree.stats.total_files),
        );

        emit(&app, "Clasificando proyectos...", 65, None, None, None);
        let known_brands = shared.brands.load();
        let templates = shared.templates.load();
        let (proposals, summary) =
            project_analyzer::analyze_tree(&tree, template_id.as_deref(), &known_brands, &templates, &shared.logger);

        emit(&app, "Generando propuestas...", 85, None, None, None);
        std::thread::sleep(std::time::Duration::from_millis(300));

        let tree_summary = TreeSummary {
            name: tree.name.clone(),
            // Send every top-level child — the frontend itself slices to the
            // first 40 for display (see renderCurrentTree in index.html).
            children: tree
                .children
                .iter()
                .map(|c| TreeChildSummary { name: c.name.clone(), path: c.path.clone(), stats: c.stats.clone(), child_count: c.children.len() })
                .collect(),
            stats: tree.stats.clone(),
        };

        Ok(serde_json::json!({
            "proposals": proposals,
            "summary": summary,
            "resolvedPath": folder_path,
            "forcedTemplateId": template_id,
            "tree": tree_summary,
        }))
    })
    .await
    .map_err(to_str_err)?
}

// ── Execute / undo ───────────────────────────────────────────────────
#[tauri::command]
pub async fn execute(proposals: Vec<Proposal>, dest_root: String, state: State<'_, Shared>) -> Result<ApplyResult, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<ApplyResult, String> {
        Ok(shared.execution.apply(&proposals, Path::new(&dest_root), &shared.logger))
    })
    .await
    .map_err(to_str_err)?
}

#[tauri::command]
pub async fn undo(state: State<'_, Shared>) -> Result<UndoResult, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<UndoResult, String> { Ok(shared.execution.undo(&shared.logger)) })
        .await
        .map_err(to_str_err)?
}

// ── Brands ───────────────────────────────────────────────────────────
#[tauri::command]
pub fn brands_list(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "known": state.brands.load() }))
}

#[tauri::command]
pub fn brands_add(brand: BrandInput, state: State<'_, Shared>) -> Result<Brand, String> {
    state.brands.add(brand)
}

#[tauri::command]
pub fn brands_update(id: String, patch: BrandPatch, state: State<'_, Shared>) -> Result<Brand, String> {
    state.brands.update(&id, patch)
}

#[tauri::command]
pub fn brands_delete(id: String, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    state.brands.remove(&id)?;
    Ok(serde_json::json!({ "ok": true }))
}

// ── Templates ("jerarquías") ─────────────────────────────────────────
#[tauri::command]
pub fn templates_list(state: State<'_, Shared>) -> Result<Vec<Template>, String> {
    Ok(state.templates.all())
}

#[tauri::command]
pub fn templates_add(template: serde_json::Value, state: State<'_, Shared>) -> Result<Template, String> {
    state.templates.add_template(template)
}

#[tauri::command]
pub fn templates_update(id: String, template: serde_json::Value, state: State<'_, Shared>) -> Result<Template, String> {
    state.templates.update_template(&id, template)
}

#[tauri::command]
pub fn templates_delete(id: String, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    state.templates.delete_template(&id)?;
    Ok(serde_json::json!({ "ok": true }))
}

// ── Finder integration ──────────────────────────────────────────────
#[tauri::command]
pub fn open_path(path: String) -> Result<serde_json::Value, String> {
    Command::new("open").arg(&path).status().map_err(to_str_err)?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub fn reveal_path(path: String) -> Result<serde_json::Value, String> {
    Command::new("open").arg("-R").arg(&path).status().map_err(to_str_err)?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub fn choose_folder() -> Result<serde_json::Value, String> {
    let script = r#"POSIX path of (choose folder with prompt "Selecciona la carpeta a organizar")"#;
    let output = Command::new("osascript").arg("-e").arg(script).output().map_err(to_str_err)?;

    if output.status.success() {
        let mut path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        Ok(serde_json::json!({ "canceled": false, "path": path }))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("-128") {
            Ok(serde_json::json!({ "canceled": true }))
        } else {
            Err(format!("No se pudo abrir el selector de carpetas: {}", stderr.trim()))
        }
    }
}

// ── Misc / logs ──────────────────────────────────────────────────────
#[tauri::command]
pub fn ping(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "ok": true, "version": "2.0.0", "hasUndo": state.execution.has_undo() }))
}

#[tauri::command]
pub fn logs_recent(limit: Option<usize>, level: Option<String>, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    let entries = state.logger.get_recent(limit.unwrap_or(200), level.as_deref());
    Ok(serde_json::json!({ "entries": entries, "filePath": state.logger.log_file_path().to_string_lossy() }))
}

#[tauri::command]
pub fn logs_open(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    Command::new("open").arg("-R").arg(state.logger.log_file_path()).status().map_err(to_str_err)?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub fn logs_clear(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    state.logger.clear();
    Ok(serde_json::json!({ "ok": true }))
}
