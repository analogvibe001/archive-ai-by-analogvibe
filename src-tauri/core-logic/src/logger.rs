//! Port of src/core/logger.js — in-memory ring buffer + rotating log file.
//!
//! Behavior preserved from the original:
//! - keeps the last MAX_MEMORY_ENTRIES entries in memory (newest last internally,
//!   `get_recent` returns newest-first like the JS version)
//! - appends every entry as one JSON line to `archive-ai.log`
//! - rotates the file once it exceeds 5MB, keeping the 3 most recent rotated files
//! - logging failures never panic/crash the app — mirrors the JS try/catch-and-continue

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_MEMORY_ENTRIES: usize = 500;
const MAX_LOG_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub struct Logger {
    log_dir: PathBuf,
    log_file: PathBuf,
    buffer: Mutex<Vec<LogEntry>>,
}

impl Logger {
    pub fn new(log_dir: PathBuf) -> Self {
        let log_file = log_dir.join("archive-ai.log");
        Logger {
            log_dir,
            log_file,
            buffer: Mutex::new(Vec::new()),
        }
    }

    fn ensure_log_dir(&self) {
        let _ = fs::create_dir_all(&self.log_dir);
    }

    fn rotate_if_needed(&self) {
        let meta = match fs::metadata(&self.log_file) {
            Ok(m) => m,
            Err(_) => return, // file doesn't exist yet, nothing to rotate
        };
        if meta.len() <= MAX_LOG_FILE_BYTES {
            return;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let rotated = self.log_dir.join(format!("archive-ai.{}.log", now_ms));
        if fs::rename(&self.log_file, &rotated).is_err() {
            return;
        }
        // Keep only the 3 most recent rotated logs
        if let Ok(entries) = fs::read_dir(&self.log_dir) {
            let mut files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|f| f.starts_with("archive-ai.") && f.ends_with(".log") && f != "archive-ai.log")
                .collect();
            files.sort();
            while files.len() > 3 {
                let f = files.remove(0);
                let _ = fs::remove_file(self.log_dir.join(f));
            }
        }
    }

    fn write(&self, level: &str, category: &str, message: &str, details: Option<serde_json::Value>) -> LogEntry {
        let entry = LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.to_string(),
            category: category.to_string(),
            message: message.to_string(),
            details,
        };

        {
            let mut buf = self.buffer.lock().unwrap();
            buf.push(entry.clone());
            if buf.len() > MAX_MEMORY_ENTRIES {
                buf.remove(0);
            }
        }

        self.ensure_log_dir();
        self.rotate_if_needed();
        if let Ok(line) = serde_json::to_string(&entry) {
            use std::io::Write;
            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.log_file) {
                let _ = writeln!(f, "{}", line);
            }
        }

        entry
    }

    pub fn info(&self, category: &str, message: &str, details: Option<serde_json::Value>) -> LogEntry {
        self.write("info", category, message, details)
    }
    pub fn warn(&self, category: &str, message: &str, details: Option<serde_json::Value>) -> LogEntry {
        self.write("warn", category, message, details)
    }
    pub fn error(&self, category: &str, message: &str, details: Option<serde_json::Value>) -> LogEntry {
        self.write("error", category, message, details)
    }

    /// Newest-first, like the JS `getRecent`.
    pub fn get_recent(&self, limit: usize, level_filter: Option<&str>) -> Vec<LogEntry> {
        let buf = self.buffer.lock().unwrap();
        let mut entries: Vec<LogEntry> = match level_filter {
            Some(lvl) => buf.iter().filter(|e| e.level == lvl).cloned().collect(),
            None => buf.clone(),
        };
        let start = entries.len().saturating_sub(limit);
        entries = entries.split_off(start);
        entries.reverse();
        entries
    }

    pub fn clear(&self) {
        self.buffer.lock().unwrap().clear();
        let _ = fs::write(&self.log_file, "");
    }

    pub fn log_file_path(&self) -> &Path {
        &self.log_file
    }
}
