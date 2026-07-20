//! Port of src/core/scanner.js — recursive folder scan into a stats-annotated tree.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use crate::logger::Logger;

static SKIP: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        ".DS_Store", ".git", "node_modules", "__MACOSX", ".Trash",
        ".Spotlight-V100", ".fseventsd", "archive-ai-reports", ".localized",
    ]
    .into_iter()
    .collect()
});

static RAW_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".arw", ".cr2", ".cr3", ".dng", ".raw", ".nef", ".orf", ".rw2", ".raf", ".pef"].into_iter().collect()
});
static PHOTO_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".jpg", ".jpeg", ".png", ".heic", ".webp", ".tiff", ".tif", ".gif", ".bmp"].into_iter().collect()
});
static VIDEO_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".mp4", ".mov", ".mxf", ".avi", ".mkv", ".braw", ".r3d", ".m4v", ".wmv", ".webm", ".flv"]
        .into_iter()
        .collect()
});
static AUDIO_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".wav", ".mp3", ".aiff", ".aif", ".flac", ".m4a", ".aac", ".ogg", ".opus"].into_iter().collect()
});
static DESIGN_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".psd", ".ai", ".indd", ".sketch", ".fig", ".xd", ".afdesign", ".afphoto", ".eps", ".svg"]
        .into_iter()
        .collect()
});
static MOTION_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [".aep", ".aet", ".mogrt", ".moho", ".blend", ".c4d", ".prproj", ".ppj", ".fcpbundle"]
        .into_iter()
        .collect()
});
static DOC_EXT: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        ".pdf", ".docx", ".doc", ".xlsx", ".xls", ".pptx", ".ppt", ".txt", ".md", ".csv", ".pages",
        ".numbers", ".key",
    ]
    .into_iter()
    .collect()
});

pub fn get_file_type(ext: &str) -> &'static str {
    if RAW_EXT.contains(ext) {
        "raw_photo"
    } else if PHOTO_EXT.contains(ext) {
        "photo"
    } else if VIDEO_EXT.contains(ext) {
        "video"
    } else if AUDIO_EXT.contains(ext) {
        "audio"
    } else if DESIGN_EXT.contains(ext) {
        "design"
    } else if MOTION_EXT.contains(ext) {
        "motion"
    } else if DOC_EXT.contains(ext) {
        "document"
    } else {
        "other"
    }
}

pub fn format_size(b: u64) -> String {
    if b < 1024 {
        format!("{}B", b)
    } else if b < 1_048_576 {
        format!("{:.1}KB", b as f64 / 1024.0)
    } else if b < 1_073_741_824 {
        format!("{:.1}MB", b as f64 / 1_048_576.0)
    } else {
        format!("{:.2}GB", b as f64 / 1_073_741_824.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DateInfo {
    pub year: i32,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<i32>,
}

// ── extractDateFromName ──────────────────────────────────────────────
static RE_YMD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4})[-_](\d{2})[-_](\d{2})").unwrap());
static RE_DMY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{2})[-_](\d{2})[-_](\d{4})").unwrap());
static RE_COMPACT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4})(\d{2})(\d{2})").unwrap());
static RE_YEAR_MONTHNAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\d{4})[-_ ]?(enero|febrero|marzo|abril|mayo|junio|julio|agosto|septiembre|octubre|noviembre|diciembre)").unwrap()
});
static RE_MONTHNAME_YEAR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(enero|febrero|marzo|abril|mayo|junio|julio|agosto|septiembre|octubre|noviembre|diciembre)[-_ ]?(\d{4})").unwrap()
});

fn month_es_to_num(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "enero" => Some(1),
        "febrero" => Some(2),
        "marzo" => Some(3),
        "abril" => Some(4),
        "mayo" => Some(5),
        "junio" => Some(6),
        "julio" => Some(7),
        "agosto" => Some(8),
        "septiembre" => Some(9),
        "octubre" => Some(10),
        "noviembre" => Some(11),
        "diciembre" => Some(12),
        _ => None,
    }
}

/// Mirrors extractDateFromName(name) in scanner.js exactly, including pattern
/// priority order and the 2010-2030 sanity range.
pub fn extract_date_from_name(name: &str) -> Option<DateInfo> {
    if let Some(m) = RE_YMD.captures(name) {
        let y: i32 = m[1].parse().ok()?;
        let mn: u32 = m[2].parse().ok()?;
        let d: u32 = m[3].parse().ok()?;
        if (2010..=2030).contains(&y) && (1..=12).contains(&mn) && (1..=31).contains(&d) {
            return Some(DateInfo { year: y, month: Some(mn), day: Some(d), source: "name".into(), confidence: None });
        }
    }
    if let Some(m) = RE_DMY.captures(name) {
        let d: u32 = m[1].parse().ok()?;
        let mn: u32 = m[2].parse().ok()?;
        let y: i32 = m[3].parse().ok()?;
        if (2010..=2030).contains(&y) && (1..=12).contains(&mn) && (1..=31).contains(&d) {
            return Some(DateInfo { year: y, month: Some(mn), day: Some(d), source: "name".into(), confidence: None });
        }
    }
    if let Some(m) = RE_COMPACT.captures(name) {
        let y: i32 = m[1].parse().ok()?;
        let mn: u32 = m[2].parse().ok()?;
        let d: u32 = m[3].parse().ok()?;
        if (2010..=2030).contains(&y) && (1..=12).contains(&mn) && (1..=31).contains(&d) {
            return Some(DateInfo { year: y, month: Some(mn), day: Some(d), source: "name".into(), confidence: None });
        }
    }
    if let Some(m) = RE_YEAR_MONTHNAME.captures(name) {
        if let Some(mon) = month_es_to_num(&m[2]) {
            let y: i32 = m[1].parse().ok()?;
            return Some(DateInfo { year: y, month: Some(mon), day: None, source: "name".into(), confidence: None });
        }
    }
    if let Some(m) = RE_MONTHNAME_YEAR.captures(name) {
        if let Some(mon) = month_es_to_num(&m[1]) {
            let y: i32 = m[2].parse().ok()?;
            if (2010..=2030).contains(&y) {
                return Some(DateInfo { year: y, month: Some(mon), day: None, source: "name".into(), confidence: None });
            }
        }
    }
    None
}

// ── Tree types ────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    #[serde(rename = "totalFiles")]
    pub total_files: u64,
    #[serde(rename = "totalSize")]
    pub total_size: u64,
    pub photos: u64,
    #[serde(rename = "rawPhotos")]
    pub raw_photos: u64,
    pub videos: u64,
    pub audio: u64,
    pub design: u64,
    pub motion: u64,
    pub documents: u64,
    pub other: u64,
    #[serde(rename = "totalSizeFormatted", skip_serializing_if = "Option::is_none")]
    pub total_size_formatted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub id: String,
    pub name: String,
    pub ext: String,
    #[serde(rename = "type")]
    pub file_type: String,
    #[serde(rename = "fullPath")]
    pub full_path: String,
    pub size: u64,
    #[serde(rename = "sizeFormatted")]
    pub size_formatted: String,
    pub mtime: f64,
    #[serde(rename = "mtimeDate")]
    pub mtime_date: DateInfo,
    #[serde(rename = "nameDate")]
    pub name_date: Option<DateInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    pub depth: u32,
    pub children: Vec<TreeNode>,
    pub files: Vec<FileNode>,
    pub stats: Stats,
    pub dates: Vec<DateInfo>,
    #[serde(rename = "nameDate")]
    pub name_date: Option<DateInfo>,
}

fn ext_lower(name: &str) -> String {
    match Path::new(name).extension() {
        Some(e) => format!(".{}", e.to_string_lossy().to_lowercase()),
        None => String::new(),
    }
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn system_time_to_date(t: SystemTime) -> DateInfo {
    use chrono::{DateTime, Datelike, Local, Utc};
    let utc: DateTime<Utc> = t.into();
    let local: DateTime<Local> = utc.with_timezone(&Local);
    DateInfo {
        year: local.year(),
        month: Some(local.month()),
        day: Some(local.day()),
        source: "mtime".into(),
        confidence: None,
    }
}

fn scan_dir(dir_path: &Path, root_path: &Path, depth: u32, logger: &Logger) -> TreeNode {
    let name = dir_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let relative_path = pathdiff::diff_paths(dir_path, root_path)
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string());

    let mut result = TreeNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        path: dir_path.to_string_lossy().to_string(),
        relative_path,
        depth,
        children: Vec::new(),
        files: Vec::new(),
        stats: Stats::default(),
        dates: Vec::new(),
        name_date: extract_date_from_name(&name),
    };

    let entries = match std::fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(e) => {
            logger.warn(
                "scanner",
                &format!("No se pudo leer la carpeta: {}", dir_path.display()),
                Some(serde_json::json!({ "error": e.to_string() })),
            );
            return result;
        }
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if SKIP.contains(fname.as_str()) || fname.starts_with('.') {
            continue;
        }
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ftype.is_dir() {
            dirs.push(entry.path());
        } else if ftype.is_file() {
            files.push((fname, entry.path()));
        }
    }
    dirs.sort();
    files.sort();

    for child_path in dirs {
        let child = scan_dir(&child_path, root_path, depth + 1, logger);
        result.stats.total_files += child.stats.total_files;
        result.stats.total_size += child.stats.total_size;
        result.stats.photos += child.stats.photos;
        result.stats.raw_photos += child.stats.raw_photos;
        result.stats.videos += child.stats.videos;
        result.stats.audio += child.stats.audio;
        result.stats.design += child.stats.design;
        result.stats.motion += child.stats.motion;
        result.stats.documents += child.stats.documents;
        result.stats.other += child.stats.other;
        result.dates.extend(child.dates.clone());
        result.children.push(child);
    }

    for (fname, full_path) in files {
        let meta = match std::fs::metadata(&full_path) {
            Ok(m) => m,
            Err(e) => {
                logger.warn(
                    "scanner",
                    &format!("No se pudo leer el archivo: {}", full_path.display()),
                    Some(serde_json::json!({ "error": e.to_string() })),
                );
                continue;
            }
        };
        if meta.len() == 0 {
            continue;
        }
        let ext = ext_lower(&fname);
        let ftype = get_file_type(&ext);
        let file_date = extract_date_from_name(&stem(&fname));
        let mtime_date = meta.modified().map(system_time_to_date).unwrap_or(DateInfo {
            year: 1970,
            month: Some(1),
            day: Some(1),
            source: "mtime".into(),
            confidence: None,
        });
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0);

        let file = FileNode {
            id: uuid::Uuid::new_v4().to_string(),
            name: fname.clone(),
            ext: ext.clone(),
            file_type: ftype.to_string(),
            full_path: full_path.to_string_lossy().to_string(),
            size: meta.len(),
            size_formatted: format_size(meta.len()),
            mtime: mtime_ms,
            mtime_date: mtime_date.clone(),
            name_date: file_date.clone(),
        };

        result.stats.total_files += 1;
        result.stats.total_size += meta.len();
        match ftype {
            "raw_photo" => result.stats.raw_photos += 1,
            "photo" => result.stats.photos += 1,
            "video" => result.stats.videos += 1,
            "audio" => result.stats.audio += 1,
            "design" => result.stats.design += 1,
            "motion" => result.stats.motion += 1,
            "document" => result.stats.documents += 1,
            _ => result.stats.other += 1,
        }

        let d = file_date.or(Some(mtime_date));
        if let Some(d) = d {
            result.dates.push(d);
        }
        result.files.push(file);
    }

    result.stats.total_size_formatted = Some(format_size(result.stats.total_size));
    result
}

pub fn scan(folder_path: &Path, logger: &Logger) -> Result<TreeNode, String> {
    if !folder_path.exists() {
        return Err(format!("Carpeta no encontrada: {}", folder_path.display()));
    }
    Ok(scan_dir(folder_path, folder_path, 0, logger))
}

fn count_dirs(node: &TreeNode) -> u64 {
    1 + node.children.iter().map(count_dirs).sum::<u64>()
}

pub fn count_all_dirs(node: &TreeNode) -> u64 {
    count_dirs(node)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickStats {
    #[serde(rename = "folderCount")]
    pub folder_count: i64,
    #[serde(rename = "fileCount")]
    pub file_count: u64,
    #[serde(rename = "totalSize")]
    pub total_size: String,
    pub photos: u64,
    pub videos: u64,
}

pub fn quick_stats(folder_path: &Path, logger: &Logger) -> Result<QuickStats, String> {
    let tree = scan(folder_path, logger)?;
    Ok(QuickStats {
        folder_count: count_dirs(&tree) as i64 - 1,
        file_count: tree.stats.total_files,
        total_size: tree.stats.total_size_formatted.clone().unwrap_or_default(),
        photos: tree.stats.photos + tree.stats.raw_photos,
        videos: tree.stats.videos,
    })
}

// ── flattenFiles ─────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct FlatFile {
    pub file: FileNode,
    pub dir_path: String,
    pub dir_relative: String,
    pub ancestor_chain: Vec<String>,
    pub ancestor_path: String,
}

pub fn flatten_files(node: &TreeNode) -> Vec<FlatFile> {
    fn walk(node: &TreeNode, ancestor_chain: &[String], out: &mut Vec<FlatFile>) {
        let mut chain = ancestor_chain.to_vec();
        chain.push(node.name.clone());

        for f in &node.files {
            out.push(FlatFile {
                file: f.clone(),
                dir_path: node.path.clone(),
                dir_relative: node.relative_path.clone(),
                ancestor_chain: chain.clone(),
                ancestor_path: if chain.len() > 1 { chain[1..].join("/") } else { ".".to_string() },
            });
        }
        for child in &node.children {
            walk(child, &chain, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &[], &mut out);
    out
}
