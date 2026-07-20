//! Port of src/core/executionEngine.js — moves files/folders per approved
//! proposals, relocates emptied-out branches to "_Revisar y borrar" instead
//! of deleting them, and records everything so it can be undone.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::logger::Logger;
use crate::project_analyzer::Proposal;

const REVIEW_FOLDER_NAME: &str = "_Revisar y borrar";

fn ignorable_junk() -> HashSet<&'static str> {
    [".DS_Store", ".localized", "Thumbs.db", "desktop.ini", ".Spotlight-V100", ".fseventsd", ".Trash"]
        .into_iter()
        .collect()
}

fn resolve_conflict(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let dir = dest.parent().unwrap_or_else(|| Path::new("."));
    let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let base = dest.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    for i in 2..1000 {
        let candidate = dir.join(format!("{} ({}){}", base, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    let ts = chrono::Utc::now().timestamp_millis();
    PathBuf::from(format!("{}_{}", dest.to_string_lossy(), ts))
}

fn move_file(src: &Path, dest: &Path) -> Result<PathBuf, String> {
    if !src.exists() {
        return Err(format!("Archivo no encontrado: {}", src.display()));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let final_dest = resolve_conflict(dest);
    if fs::rename(src, &final_dest).is_err() {
        fs::copy(src, &final_dest).map_err(|e| e.to_string())?;
        fs::remove_file(src).map_err(|e| e.to_string())?;
    }
    Ok(final_dest)
}

fn move_folder(src: &Path, dest: &Path) -> Result<PathBuf, String> {
    if !src.exists() {
        return Err(format!("Carpeta no encontrada: {}", src.display()));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let final_dest = resolve_conflict(dest);
    fs::rename(src, &final_dest).map_err(|e| e.to_string())?;
    Ok(final_dest)
}

fn is_recursively_empty(dir_path: &Path) -> bool {
    let junk = ignorable_junk();
    let entries = match fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if junk.contains(name.as_str()) {
            continue;
        }
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => return false,
        };
        if ftype.is_dir() {
            if !is_recursively_empty(&entry.path()) {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

fn find_top_empty_ancestor(dir_path: &Path, root_path: &Path) -> Option<PathBuf> {
    if !is_recursively_empty(dir_path) {
        return None;
    }
    let mut top = dir_path.to_path_buf();
    loop {
        let parent = match top.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
        if parent == root_path || !parent.starts_with(root_path) || !is_recursively_empty(&parent) {
            break;
        }
        top = parent;
    }
    Some(top)
}

#[derive(Debug, Clone)]
struct ArchivedFolder {
    from: String,
    to: String,
}

fn archive_empty_branches(
    touched_source_dirs: &HashSet<String>,
    root_path: &Path,
    dest_root: &Path,
    archived: &mut Vec<ArchivedFolder>,
    logger: &Logger,
) {
    let mut branch_roots: HashSet<PathBuf> = HashSet::new();
    for dir in touched_source_dirs {
        if let Some(top) = find_top_empty_ancestor(Path::new(dir), root_path) {
            branch_roots.insert(top);
        }
    }

    for top in branch_roots {
        if !top.exists() {
            continue; // already swallowed by another branch's move
        }
        let relative = pathdiff::diff_paths(&top, root_path).unwrap_or_else(|| top.clone());
        let review_dest = resolve_conflict(&dest_root.join(REVIEW_FOLDER_NAME).join(&relative));
        if let Some(parent) = review_dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::rename(&top, &review_dest) {
            Ok(_) => {
                archived.push(ArchivedFolder { from: top.to_string_lossy().to_string(), to: review_dest.to_string_lossy().to_string() });
                logger.info(
                    "execution",
                    &format!("Carpeta vacía movida a revisión: {}", relative.display()),
                    Some(serde_json::json!({ "from": top.to_string_lossy(), "to": review_dest.to_string_lossy() })),
                );
            }
            Err(e) => {
                logger.warn(
                    "execution",
                    &format!("No se pudo mover carpeta vacía a revisión: {}", top.display()),
                    Some(serde_json::json!({ "error": e.to_string() })),
                );
            }
        }
    }
}

fn sweep_empty_folders(dir_path: &Path, dest_root: &Path, review_folder_path: &Path, archived: &mut Vec<ArchivedFolder>, logger: &Logger) {
    let entries = match fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ftype.is_dir() {
            continue;
        }
        let full = entry.path();
        if full == review_folder_path {
            continue;
        }

        if is_recursively_empty(&full) {
            let relative = pathdiff::diff_paths(&full, dest_root).unwrap_or_else(|| full.clone());
            let review_dest = resolve_conflict(&review_folder_path.join(&relative));
            if let Some(parent) = review_dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::rename(&full, &review_dest) {
                Ok(_) => {
                    archived.push(ArchivedFolder { from: full.to_string_lossy().to_string(), to: review_dest.to_string_lossy().to_string() });
                    logger.info(
                        "execution",
                        &format!("Carpeta vacía adicional movida a revisión: {}", relative.display()),
                        Some(serde_json::json!({ "from": full.to_string_lossy(), "to": review_dest.to_string_lossy() })),
                    );
                }
                Err(e) => {
                    logger.warn(
                        "execution",
                        &format!("No se pudo mover carpeta vacía a revisión: {}", full.display()),
                        Some(serde_json::json!({ "error": e.to_string() })),
                    );
                }
            }
        } else {
            sweep_empty_folders(&full, dest_root, review_folder_path, archived, logger);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MoveError {
    pub file: String,
    pub group: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub moved: usize,
    pub errors: usize,
    #[serde(rename = "errorList")]
    pub error_list: Vec<MoveError>,
    #[serde(rename = "archivedFolders")]
    pub archived_folders: usize,
    #[serde(rename = "reviewFolder")]
    pub review_folder: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UndoResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoveOp {
    #[serde(rename = "type")]
    kind: String,
    src: String,
    dest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UndoRecord {
    id: String,
    timestamp: i64,
    #[serde(rename = "destRoot")]
    dest_root: String,
    ops: Vec<MoveOp>,
    #[serde(rename = "archivedFolders")]
    archived_folders: Vec<MoveOp>,
}

pub struct ExecutionEngine {
    undo_file: PathBuf,
}

impl ExecutionEngine {
    pub fn new(undo_file: PathBuf) -> Self {
        ExecutionEngine { undo_file }
    }

    pub fn has_undo(&self) -> bool {
        self.undo_file.exists()
    }

    pub fn apply(&self, proposals: &[Proposal], dest_root: &Path, logger: &Logger) -> ApplyResult {
        let mut ops: Vec<MoveOp> = Vec::new();
        let mut errors: Vec<MoveError> = Vec::new();
        let mut touched_source_dirs: HashSet<String> = HashSet::new();

        let approved_count = proposals.iter().filter(|p| p.status == "accepted" || p.status == "changed").count();
        logger.info(
            "execution",
            &format!("Iniciando ejecución: {} grupos aprobados de {} totales", approved_count, proposals.len()),
            Some(serde_json::json!({ "destRoot": dest_root.to_string_lossy() })),
        );

        for proposal in proposals {
            if proposal.status != "accepted" && proposal.status != "changed" {
                continue;
            }
            let base_override = if proposal.status == "changed" { proposal.user_path_override.clone() } else { None };

            for file in &proposal.files {
                let dest_dir = if let Some(base) = &base_override {
                    let base_path = Path::new(base);
                    if base_path.is_absolute() {
                        base_path.to_path_buf()
                    } else {
                        dest_root.join(base_path)
                    }
                } else {
                    dest_root.join(&file.destination_path)
                };
                let dest_path = dest_dir.join(&file.file_name);
                let src_path = Path::new(&file.source_path);

                match move_file(src_path, &dest_path) {
                    Ok(final_dest) => {
                        ops.push(MoveOp { kind: "file".into(), src: file.source_path.clone(), dest: final_dest.to_string_lossy().to_string() });
                        touched_source_dirs.insert(file.source_folder.clone());
                    }
                    Err(e) => {
                        logger.error(
                            "execution",
                            &format!("Error moviendo archivo: {}", file.file_name),
                            Some(serde_json::json!({ "group": proposal.label, "sourcePath": file.source_path, "error": e })),
                        );
                        errors.push(MoveError { file: file.file_name.clone(), group: proposal.label.clone(), error: e });
                    }
                }
            }
        }

        let mut archived_folders: Vec<ArchivedFolder> = Vec::new();
        archive_empty_branches(&touched_source_dirs, dest_root, dest_root, &mut archived_folders, logger);

        let review_folder_path = dest_root.join(REVIEW_FOLDER_NAME);
        sweep_empty_folders(dest_root, dest_root, &review_folder_path, &mut archived_folders, logger);

        if !ops.is_empty() {
            let record = UndoRecord {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now().timestamp_millis(),
                dest_root: dest_root.to_string_lossy().to_string(),
                ops: ops.iter().map(|o| MoveOp { kind: "file".into(), src: o.dest.clone(), dest: o.src.clone() }).collect(),
                archived_folders: archived_folders.iter().map(|a| MoveOp { kind: "folder".into(), src: a.to.clone(), dest: a.from.clone() }).collect(),
            };
            if let Ok(json) = serde_json::to_string_pretty(&record) {
                let _ = fs::write(&self.undo_file, json);
            }
        }

        logger.info(
            "execution",
            &format!(
                "Ejecución completada: {} archivos movidos, {} carpetas vacías movidas a revisión, {} errores",
                ops.len(),
                archived_folders.len(),
                errors.len()
            ),
            None,
        );

        ApplyResult {
            moved: ops.len(),
            errors: errors.len(),
            error_list: errors,
            archived_folders: archived_folders.len(),
            review_folder: if !archived_folders.is_empty() { Some(review_folder_path.to_string_lossy().to_string()) } else { None },
        }
    }

    pub fn undo(&self, logger: &Logger) -> UndoResult {
        let record: UndoRecord = match fs::read_to_string(&self.undo_file).ok().and_then(|s| serde_json::from_str(&s).ok()) {
            Some(r) => r,
            None => return UndoResult { success: false, restored: None, errors: None, error: Some("No hay operación para deshacer.".to_string()) },
        };

        logger.info(
            "execution",
            &format!("Iniciando deshacer: {} archivos y {} carpetas a revertir", record.ops.len(), record.archived_folders.len()),
            None,
        );

        let mut restored = 0usize;
        let mut errors = 0usize;

        for op in record.archived_folders.iter().rev() {
            match move_folder(Path::new(&op.src), Path::new(&op.dest)) {
                Ok(_) => restored += 1,
                Err(e) => {
                    errors += 1;
                    logger.error("execution", "Error al deshacer movimiento de carpeta", Some(serde_json::json!({ "src": op.src, "dest": op.dest, "error": e })));
                }
            }
        }

        for op in record.ops.iter().rev() {
            match move_file(Path::new(&op.src), Path::new(&op.dest)) {
                Ok(_) => restored += 1,
                Err(e) => {
                    errors += 1;
                    logger.error("execution", "Error al deshacer movimiento", Some(serde_json::json!({ "src": op.src, "dest": op.dest, "error": e })));
                }
            }
        }

        let _ = fs::remove_file(&self.undo_file);

        if !record.dest_root.is_empty() {
            let review_folder = Path::new(&record.dest_root).join(REVIEW_FOLDER_NAME);
            if let Ok(entries) = fs::read_dir(&review_folder) {
                if entries.count() == 0 {
                    let _ = fs::remove_dir(&review_folder);
                }
            }
        }

        logger.info("execution", &format!("Deshacer completado: {} restaurados, {} errores", restored, errors), None);

        UndoResult { success: true, restored: Some(restored), errors: Some(errors), error: None }
    }
}
