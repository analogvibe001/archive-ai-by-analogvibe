//! Port of src/core/projectAnalyzer.js — the core classification engine.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::brand_detector::{detect_from_chain, suggest_new_brand, Brand};
use crate::date_extractor::{best_date_for_file, month_name};
use crate::logger::Logger;
use crate::scanner::{extract_date_from_name, flatten_files, DateInfo, FileNode, TreeNode};
use crate::template_engine::{fill_path, Template};

static IGNORE_FOLDER_TOKENS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "fotos", "videos", "audio", "recursos", "footage", "clips", "clip", "reels",
        "reel", "pendiente", "pendientes", "nueva", "tanda", "sin", "identificar",
        "desconocido", "download", "downloads", "img", "get", "light", "final",
        "editado", "editados", "editada", "editadas", "raw", "assets", "exports",
        "editable", "backup", "temp", "tmp", "old", "copy", "otros", "varios", "misc",
    ]
    .into_iter()
    .collect()
});

// ── Camera-original filename detection ──────────────────────────────
static CAMERA_SEGMENT_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"^[A-Z]\d{3,5}$").unwrap(),
        Regex::new(r"^[A-Z]{1,3}\d{2,4}[A-Z]\d{2,4}$").unwrap(),
        Regex::new(r"(?i)^MVI\d{4,5}$").unwrap(),
        Regex::new(r"(?i)^DSCF?\d{4,6}$").unwrap(),
        Regex::new(r"(?i)^GOPR\d{4}$").unwrap(),
        Regex::new(r"(?i)^G[HXL]\d{6}$").unwrap(),
        Regex::new(r"(?i)^IMG\d{4,6}$").unwrap(),
        Regex::new(r"^P\d{7,8}$").unwrap(),
        Regex::new(r"(?i)^[A-Z]\d{3}C\d{3,4}$").unwrap(),
        Regex::new(r"(?i)^C\d{3,5}$").unwrap(),
        Regex::new(r"(?i)^[A-Z]\d{6,10}$").unwrap(),
        Regex::new(r"(?i)^SRG\d{4,6}$").unwrap(),
        Regex::new(r"(?i)^PICT\d{4,6}$").unwrap(),
        Regex::new(r"(?i)^VID_\d{8}_\d{6}$").unwrap(),
        Regex::new(r"(?i)^PXL_\d{8}_\d{9}[A-Z]{2}$").unwrap(),
    ]
});

static CAMERA_FULLNAME_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"^\d{8}_\d{6}$").unwrap(),
        Regex::new(r"(?i)^clip[-_ ]?\d{2,5}$").unwrap(),
    ]
});

static UNAMBIGUOUS_CAMERA_PREFIXES: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)^DJI[_\-]?\d").unwrap(),
        Regex::new(r"(?i)^GOPR").unwrap(),
        Regex::new(r"(?i)^GX\d{2}").unwrap(),
        Regex::new(r"(?i)^MVI[_\-]?\d").unwrap(),
        Regex::new(r"(?i)^DSCF?\d").unwrap(),
        Regex::new(r"(?i)^PXL[_\-]").unwrap(),
    ]
});

static LOG_PROFILE_HINTS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)\bs-?log-?[23]?\b").unwrap(),
        Regex::new(r"(?i)\bc-?log-?[23]?\b").unwrap(),
        Regex::new(r"(?i)\bv-?log\b").unwrap(),
        Regex::new(r"(?i)\blog-?c\b").unwrap(),
        Regex::new(r"(?i)\bn-?log\b").unwrap(),
        Regex::new(r"(?i)\bf-?log-?2?\b").unwrap(),
        Regex::new(r"(?i)\bredlog-?f?\b").unwrap(),
        Regex::new(r"(?i)\bd-?log-?m?\b").unwrap(),
        Regex::new(r"(?i)\bhlg\b").unwrap(),
        Regex::new(r"(?i)(?:^|[_\-\s])log(?:$|[_\-\s])").unwrap(),
    ]
});

static SUFFIX_NOISE_TOKENS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(4k|2k|8k|hd|uhd|fhd|prores|dnxhd|422|444|hq|proxy|flat|raw)$").unwrap());

static EDIT_NAME_HINTS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)\bv\d+\b").unwrap(),
        Regex::new(r"(?i)\bversion\b").unwrap(),
        Regex::new(r"(?i)\bsolo sube\b").unwrap(),
        Regex::new(r"(?i)\bno mires\b").unwrap(),
        Regex::new(r"(?i)\bfinal\b").unwrap(),
        Regex::new(r"(?i)\bcorregid[oa]\b").unwrap(),
        Regex::new(r"(?i)\baprobad[oa]\b").unwrap(),
        Regex::new(r"(?i)\bexportad[oa]\b").unwrap(),
        Regex::new(r"(?i)\bedit(ado|ada|ed)?\b").unwrap(),
        Regex::new(r"(?i)\bsubir\b").unwrap(),
        Regex::new(r"(?i)\brevisi[oó]n\b").unwrap(),
        Regex::new(r"(?i)\bentrega\b").unwrap(),
        Regex::new(r"(?i)\bcliente\b").unwrap(),
        Regex::new(r"(?i)\breel\b").unwrap(),
        Regex::new(r"(?i)\bteaser\b").unwrap(),
        Regex::new(r"(?i)\bpromo\b").unwrap(),
        Regex::new(r"(?i)\btrailer\b").unwrap(),
    ]
});

static RAW_ONLY_VIDEO_EXT: Lazy<HashSet<&'static str>> =
    Lazy::new(|| [".braw", ".r3d", ".ari", ".arx"].into_iter().collect());

static RE_TRAILING_COPY_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\(\d+\)\s*$").unwrap());
static RE_SEGMENT_SPLIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s_\-]+").unwrap());
static RE_LOOKS_LIKE_WORD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-ZáéíóúñÁÉÍÓÚÑ]{4,}$").unwrap());

static RE_STRONG_PROJECT_WORD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(producci|cobertura|animaci|reel|sesion|sesión|rodaje|shooting)").unwrap());
static RE_ANIMATION_HINT: Lazy<Regex> = Lazy::new(|| Regex::new(r"animaci|motion|after\s?effects|\bae\b").unwrap());
static RE_COBERTURA_HINT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"cobertura|evento|event|bts|behind|lanzamiento|launch|premiere").unwrap());

fn looks_like_camera_filename(name_no_ext: &str) -> bool {
    let trimmed = RE_TRAILING_COPY_SUFFIX.replace(name_no_ext.trim(), "").trim().to_string();

    if UNAMBIGUOUS_CAMERA_PREFIXES.iter().any(|p| p.is_match(&trimmed)) {
        return true;
    }
    if CAMERA_FULLNAME_PATTERNS.iter().any(|p| p.is_match(&trimmed)) {
        return true;
    }
    if CAMERA_SEGMENT_PATTERNS.iter().any(|p| p.is_match(&trimmed)) {
        return true;
    }

    let raw_segments: Vec<&str> = RE_SEGMENT_SPLIT.split(&trimmed).filter(|s| !s.is_empty()).collect();
    let segments: Vec<&str> = raw_segments.into_iter().filter(|s| !SUFFIX_NOISE_TOKENS.is_match(s)).collect();
    if segments.is_empty() || segments.len() > 5 {
        return false;
    }

    let matches_camera_code = |seg: &str| CAMERA_SEGMENT_PATTERNS.iter().any(|p| p.is_match(seg));

    let mut camera_segment_count = 0;
    for i in 0..segments.len() {
        if matches_camera_code(segments[i]) {
            camera_segment_count += 1;
            continue;
        }
        if i < segments.len() - 1 {
            let joined = format!("{}{}", segments[i], segments[i + 1]);
            if matches_camera_code(&joined) {
                camera_segment_count += 1;
            }
        }
    }
    if camera_segment_count == 0 {
        return false;
    }

    let looks_like_word = |s: &str| RE_LOOKS_LIKE_WORD.is_match(s) && !matches_camera_code(s);
    if segments.iter().any(|s| looks_like_word(s)) {
        return false;
    }

    true
}

fn has_log_profile_hint(name_no_ext: &str) -> bool {
    LOG_PROFILE_HINTS.iter().any(|p| p.is_match(name_no_ext))
}

fn looks_like_human_edited_name(name_no_ext: &str) -> bool {
    let has_spaces = name_no_ext.trim().chars().any(|c| c.is_whitespace());
    let has_edit_hint = EDIT_NAME_HINTS.iter().any(|p| p.is_match(name_no_ext));
    has_spaces || has_edit_hint
}

fn looks_like_resource_file(file: &FileNode) -> bool {
    file.file_type == "audio" || file.file_type == "design"
}

#[derive(Debug, Clone)]
pub struct SubBucket {
    pub branch: String,
    pub sub: String,
    pub sub_reason: Option<String>,
    pub ambiguous: bool,
}

fn file_name_stem(file: &FileNode) -> String {
    std::path::Path::new(&file.name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file.name.clone())
}

pub fn file_sub_bucket(file: &FileNode) -> SubBucket {
    if file.file_type == "raw_photo" {
        return SubBucket { branch: "Fotos".into(), sub: "Fotos RAW".into(), sub_reason: None, ambiguous: false };
    }
    if file.file_type == "photo" {
        return SubBucket { branch: "Fotos".into(), sub: "Fotos Editadas".into(), sub_reason: None, ambiguous: false };
    }
    if file.file_type == "video" {
        let name_no_ext = file_name_stem(file);

        if has_log_profile_hint(&name_no_ext) {
            return SubBucket {
                branch: "Videos".into(),
                sub: "Videos RAW".into(),
                sub_reason: Some("log_profile_name".into()),
                ambiguous: false,
            };
        }
        if looks_like_camera_filename(&name_no_ext) {
            return SubBucket {
                branch: "Videos".into(),
                sub: "Videos RAW".into(),
                sub_reason: Some("camera_name".into()),
                ambiguous: false,
            };
        }
        if looks_like_human_edited_name(&name_no_ext) {
            return SubBucket {
                branch: "Videos".into(),
                sub: "Videos Editados".into(),
                sub_reason: Some("human_name".into()),
                ambiguous: false,
            };
        }
        if RAW_ONLY_VIDEO_EXT.contains(file.ext.as_str()) {
            return SubBucket {
                branch: "Videos".into(),
                sub: "Videos RAW".into(),
                sub_reason: Some("raw_codec".into()),
                ambiguous: false,
            };
        }

        let is_cameraish = file.ext == ".mov" || file.ext == ".mxf";
        return SubBucket {
            branch: "Videos".into(),
            sub: if is_cameraish { "Videos RAW".into() } else { "Videos Editados".into() },
            sub_reason: Some(if is_cameraish { "ambiguous_camera_container".into() } else { "ambiguous_export_container".into() }),
            ambiguous: true,
        };
    }
    if file.file_type == "motion" || file.file_type == "design" {
        return SubBucket { branch: "Editable".into(), sub: "Editable".into(), sub_reason: None, ambiguous: false };
    }
    SubBucket { branch: "Otros".into(), sub: "Otros".into(), sub_reason: None, ambiguous: false }
}

fn is_raw_original(file: &FileNode) -> bool {
    if file.file_type == "raw_photo" {
        return true;
    }
    if file.file_type == "video" {
        return file_sub_bucket(file).sub == "Videos RAW";
    }
    false
}

fn dates_from_ancestors(ancestor_chain: &[String]) -> Vec<DateInfo> {
    ancestor_chain.iter().filter_map(|name| extract_date_from_name(name)).collect()
}

fn classify_file_template(file: &FileNode, chain: &[String], forced: Option<&str>, templates: &[Template]) -> String {
    if let Some(f) = forced {
        return f.to_string();
    }
    let chain_text = chain.join(" ").to_lowercase();

    if looks_like_resource_file(file) {
        let has_strong_project_word = RE_STRONG_PROJECT_WORD.is_match(&chain_text);
        if !has_strong_project_word {
            return "recursos".to_string();
        }
    }

    if file.file_type == "motion" {
        return "animaciones".to_string();
    }
    if RE_ANIMATION_HINT.is_match(&chain_text) {
        return "animaciones".to_string();
    }
    if RE_COBERTURA_HINT.is_match(&chain_text) {
        return "coberturas".to_string();
    }

    for tpl in templates.iter().filter(|t| t.custom) {
        let kws = &tpl.signals.keywords;
        if !kws.is_empty() && kws.iter().any(|kw| chain_text.contains(kw.as_str())) {
            return tpl.id.clone();
        }
    }

    if matches!(file.file_type.as_str(), "photo" | "raw_photo" | "video") {
        return "producciones".to_string();
    }
    "recursos".to_string()
}

pub struct Destination {
    pub full: String,
    pub levels: Vec<String>,
}

fn build_file_path(
    template_id: &str,
    brand: Option<&Brand>,
    date_info: Option<&DateInfo>,
    file: &FileNode,
    project_name: Option<&str>,
    templates: &[Template],
) -> Destination {
    let brand_name = brand.map(|b| b.name.clone()).unwrap_or_else(|| "Sin Marca".to_string());

    if template_id == "recursos" {
        let sub = match file.file_type.as_str() {
            "audio" => "Audio",
            "video" => "Video",
            _ => "Imágenes",
        };
        return Destination { full: format!("Recursos/{}", sub), levels: vec!["Recursos".to_string(), sub.to_string()] };
    }

    let year = date_info.map(|d| d.year).unwrap_or_else(|| {
        use chrono::Datelike;
        chrono::Local::now().year()
    });
    let month = date_info.and_then(|d| d.month);
    let day = date_info.and_then(|d| d.day);
    let month_name_s = month.map(month_name).unwrap_or("Sin Mes");
    let day_pad = day.map(|d| format!("{:02}", d)).unwrap_or_else(|| "XX".to_string());

    if let Some(custom_tpl) = templates.iter().find(|t| t.id == template_id && t.custom) {
        let mut vars: HashMap<String, String> = HashMap::new();
        vars.insert("type".into(), custom_tpl.name.clone());
        vars.insert("brand".into(), brand_name.clone());
        vars.insert("year".into(), year.to_string());
        vars.insert("month_name".into(), month_name_s.to_string());
        vars.insert("day".into(), day_pad.clone());
        vars.insert("project_name".into(), project_name.unwrap_or("Proyecto").to_string());

        let mut levels: Vec<String> = custom_tpl.levels.iter().map(|l| fill_path(l, &vars)).collect();

        if custom_tpl.split_by_media && matches!(file.file_type.as_str(), "photo" | "raw_photo" | "video") {
            let sb = file_sub_bucket(file);
            let last = levels.last().cloned().unwrap_or_default();
            levels.push(format!("{} - {}", sb.branch, last));
            if custom_tpl.split_raw_edited {
                let last2 = levels.last().cloned().unwrap_or_default();
                levels.push(format!("{} - {}", sb.sub, last2));
            }
        }

        return Destination { full: levels.join("/"), levels };
    }

    if template_id == "animaciones" {
        let root = format!("Animaciones - {} - {}", year, brand_name);
        let lvl1 = format!("{} - Animaciones - {} - {}", month_name_s, year, brand_name);
        let proj = project_name.unwrap_or("Proyecto");
        let lvl2 = format!("{} - {} - Animaciones - {} - {}", proj, month_name_s, year, brand_name);
        let branch = file_sub_bucket(file).branch;
        let sub = if branch == "Editable" {
            "Editable"
        } else if branch == "Fotos" || branch == "Videos" {
            "Exports"
        } else {
            "Assets"
        };
        let lvl3 = format!("{} - {}", sub, lvl2);
        return Destination { full: format!("{}/{}/{}/{}", root, lvl1, lvl2, lvl3), levels: vec![root, lvl1, lvl2, lvl3] };
    }

    let type_name = if template_id == "coberturas" { "Coberturas" } else { "Producciones" };
    let root = format!("{} - {} - {}", type_name, year, brand_name);
    let lvl1 = format!("{} - {} - {} - {}", month_name_s, type_name, year, brand_name);
    let lvl2 = format!("{} - {} - {} - {} - {}", day_pad, month_name_s, type_name, year, brand_name);
    let sb = file_sub_bucket(file);

    if template_id == "coberturas" {
        let lvl3 = format!("{} - {}", sb.branch, lvl2);
        return Destination { full: format!("{}/{}/{}/{}", root, lvl1, lvl2, lvl3), levels: vec![root, lvl1, lvl2, lvl3] };
    }

    let lvl3 = format!("{} - {}", sb.branch, lvl2);
    let lvl4 = format!("{} - {}", sb.sub, lvl3);
    Destination {
        full: format!("{}/{}/{}/{}/{}", root, lvl1, lvl2, lvl3, lvl4),
        levels: vec![root, lvl1, lvl2, lvl3, lvl4],
    }
}

// ── Output types (camelCase to match the frontend exactly) ───────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAnalysis {
    #[serde(rename = "fileId")]
    pub file_id: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "fileType")]
    pub file_type: String,
    #[serde(rename = "fileSize")]
    pub file_size: String,
    #[serde(rename = "sourcePath")]
    pub source_path: String,
    #[serde(rename = "sourceFolder")]
    pub source_folder: String,
    #[serde(rename = "ancestorPath")]
    pub ancestor_path: String,
    pub brand: Option<Brand>,
    #[serde(rename = "brandConfidence")]
    pub brand_confidence: i32,
    pub date: Option<DateInfo>,
    #[serde(rename = "isRaw")]
    pub is_raw: bool,
    #[serde(rename = "subBucket")]
    pub sub_bucket: Option<String>,
    #[serde(rename = "subBucketAmbiguous")]
    pub sub_bucket_ambiguous: bool,
    #[serde(rename = "templateId")]
    pub template_id: String,
    #[serde(rename = "projectName")]
    pub project_name: Option<String>,
    #[serde(rename = "destinationPath")]
    pub destination_path: String,
    #[serde(rename = "destinationLevels")]
    pub destination_levels: Vec<String>,
    pub confidence: i32,
    #[serde(rename = "needsReview")]
    pub needs_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub label: String,
    #[serde(rename = "templateId")]
    pub template_id: String,
    #[serde(rename = "templateName")]
    pub template_name: String,
    pub brand: Option<Brand>,
    pub date: Option<DateInfo>,
    #[serde(rename = "projectName")]
    pub project_name: Option<String>,
    #[serde(rename = "destinationRoot")]
    pub destination_root: String,
    #[serde(rename = "fileCount")]
    pub file_count: usize,
    #[serde(rename = "sourceFolders")]
    pub source_folders: Vec<String>,
    #[serde(rename = "sourceFolderCount")]
    pub source_folder_count: usize,
    pub confidence: i32,
    #[serde(rename = "needsReview")]
    pub needs_review: bool,
    #[serde(rename = "isResource")]
    pub is_resource: bool,
    pub files: Vec<FileAnalysis>,
    pub status: String,
    #[serde(rename = "userPathOverride")]
    pub user_path_override: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeSummary {
    #[serde(rename = "totalFolders")]
    pub total_folders: i64,
    #[serde(rename = "totalFiles")]
    pub total_files: usize,
    pub proposals: usize,
    #[serde(rename = "needsReview")]
    pub needs_review: usize,
    #[serde(rename = "highConfidence")]
    pub high_confidence: usize,
    #[serde(rename = "newBrands")]
    pub new_brands: Vec<String>,
    pub resources: usize,
}

struct Prelim {
    flat_index: usize,
    chain: Vec<String>,
    brand_result: Option<crate::brand_detector::BrandMatch>,
    own_date: Option<DateInfo>,
    template_id: String,
    sub: Option<SubBucket>,
    is_raw: bool,
    project_name: Option<String>,
    cluster_key: String,
}

pub fn analyze_tree(
    tree: &TreeNode,
    forced_template_id: Option<&str>,
    known_brands: &[Brand],
    templates: &[Template],
    logger: &Logger,
) -> (Vec<Proposal>, AnalyzeSummary) {
    let all_files = flatten_files(tree);

    logger.info(
        "analyzer",
        &format!(
            "Iniciando análisis: {} archivos encontrados{}",
            all_files.len(),
            forced_template_id.map(|f| format!(" (modo forzado: {})", f)).unwrap_or_default()
        ),
        None,
    );

    // ═══ PASS 1 ═══
    let mut prelim: Vec<Prelim> = Vec::new();
    let mut new_brand_order: Vec<String> = Vec::new();
    let mut new_brand_votes: HashMap<String, i32> = HashMap::new();

    for (idx, ff) in all_files.iter().enumerate() {
        if ff.file.size == 0 {
            continue;
        }
        let chain = &ff.ancestor_chain;
        let brand_result = detect_from_chain(chain, known_brands);
        let date_ancestors = dates_from_ancestors(&chain[1..]);
        let own_date = best_date_for_file(&ff.file, &date_ancestors);
        let template_id = classify_file_template(&ff.file, chain, forced_template_id, templates);
        let sub = if matches!(ff.file.file_type.as_str(), "video" | "photo" | "raw_photo") {
            Some(file_sub_bucket(&ff.file))
        } else {
            None
        };
        let is_raw = is_raw_original(&ff.file);

        let mut project_name: Option<String> = None;
        for i in (0..chain.len()).rev() {
            let tok = chain[i].to_lowercase();
            let alias_match = brand_result
                .as_ref()
                .map(|b| b.brand.aliases.iter().any(|a| a == &tok))
                .unwrap_or(false);
            if !IGNORE_FOLDER_TOKENS.contains(tok.as_str()) && chain[i].len() > 2 && !alias_match {
                project_name = Some(chain[i].clone());
                break;
            }
        }

        if brand_result.is_none() && template_id != "recursos" {
            let name_for_suggestion = chain.last().cloned().unwrap_or_else(|| ff.dir_path.clone());
            if let Some(s) = suggest_new_brand(&name_for_suggestion, known_brands) {
                if !new_brand_votes.contains_key(&s.suggested) {
                    new_brand_order.push(s.suggested.clone());
                }
                *new_brand_votes.entry(s.suggested).or_insert(0) += 1;
            }
        }

        let brand_key = brand_result.as_ref().map(|b| b.brand.id.clone()).unwrap_or_else(|| "sin-marca".to_string());
        let cluster_key = format!("{}|{}", template_id, brand_key);

        prelim.push(Prelim {
            flat_index: idx,
            chain: chain.clone(),
            brand_result,
            own_date,
            template_id,
            sub,
            is_raw,
            project_name,
            cluster_key,
        });
    }

    // ═══ Canonical RAW date per cluster ═══
    struct DateVote {
        date: DateInfo,
        count: i32,
    }
    let mut cluster_raw_dates: HashMap<String, Vec<(String, DateVote)>> = HashMap::new();
    for p in &prelim {
        if !p.is_raw {
            continue;
        }
        let Some(d) = &p.own_date else { continue };
        let date_key = format!(
            "{}-{}-{}",
            d.year,
            d.month.map(|m| m.to_string()).unwrap_or_else(|| "X".to_string()),
            d.day.map(|dd| dd.to_string()).unwrap_or_else(|| "X".to_string())
        );
        let entries = cluster_raw_dates.entry(p.cluster_key.clone()).or_default();
        if let Some(entry) = entries.iter_mut().find(|entry| entry.0 == date_key) {
            entry.1.count += 1;
        } else {
            entries.push((date_key, DateVote { date: d.clone(), count: 1 }));
        }
    }
    let mut canonical_date_by_cluster: HashMap<String, DateInfo> = HashMap::new();
    for (key, mut votes) in cluster_raw_dates {
        votes.sort_by(|a, b| b.1.count.cmp(&a.1.count));
        if let Some((_, best)) = votes.into_iter().next() {
            canonical_date_by_cluster.insert(key, best.date);
        }
    }

    // ═══ PASS 2 ═══
    let mut file_analyses: Vec<FileAnalysis> = Vec::new();

    for p in &prelim {
        let ff = &all_files[p.flat_index];
        let canonical_date = canonical_date_by_cluster.get(&p.cluster_key);

        let date_info: Option<DateInfo> = if p.is_raw {
            p.own_date.clone()
        } else if let Some(canonical) = canonical_date {
            let own_is_mtime_or_none = p.own_date.as_ref().map(|d| d.source == "mtime").unwrap_or(true);
            let own_is_file_name = p.own_date.as_ref().map(|d| d.source == "file_name").unwrap_or(false);
            if own_is_mtime_or_none || own_is_file_name {
                let mut d = canonical.clone();
                d.source = "inherited_from_raw".to_string();
                d.confidence = Some(std::cmp::max(70, canonical.confidence.unwrap_or(0)));
                Some(d)
            } else {
                p.own_date.clone()
            }
        } else {
            p.own_date.clone()
        };

        let brand = p.brand_result.as_ref().map(|b| b.brand.clone());
        let brand_confidence = p.brand_result.as_ref().map(|b| b.confidence).unwrap_or(0);

        let destination = build_file_path(
            &p.template_id,
            brand.as_ref(),
            date_info.as_ref(),
            &ff.file,
            p.project_name.as_deref(),
            templates,
        );
        let is_ambiguous_sub_bucket = p.sub.as_ref().map(|s| s.ambiguous).unwrap_or(false);

        let mut confidence: f64 = 35.0;
        if let Some(b) = &p.brand_result {
            confidence += b.confidence as f64 * 0.3;
        }
        if let Some(d) = &date_info {
            confidence += d.confidence.unwrap_or(0) as f64 * 0.3;
        }
        if p.template_id == "recursos" {
            confidence = confidence.max(75.0);
        }
        if is_ambiguous_sub_bucket {
            confidence -= 25.0;
        }
        let confidence = (confidence.round() as i32).clamp(5, 99);

        let needs_review = p.brand_result.is_none() || date_info.is_none() || confidence < 60 || is_ambiguous_sub_bucket;

        if needs_review {
            let mut reasons = Vec::new();
            if p.brand_result.is_none() {
                reasons.push("sin marca detectada".to_string());
            }
            if date_info.is_none() {
                reasons.push("sin fecha detectada".to_string());
            }
            if is_ambiguous_sub_bucket {
                reasons.push(format!(
                    "RAW vs Editado sin señal clara ({})",
                    p.sub.as_ref().and_then(|s| s.sub_reason.clone()).unwrap_or_default()
                ));
            }
            if confidence < 60 {
                reasons.push(format!("confianza baja ({}%)", confidence));
            }
            logger.warn(
                "analyzer",
                &format!("Archivo requiere revisión: {}", ff.file.name),
                Some(serde_json::json!({
                    "reasons": reasons, "folder": ff.dir_path, "templateId": p.template_id, "isRaw": p.is_raw
                })),
            );
        }

        file_analyses.push(FileAnalysis {
            file_id: ff.file.id.clone(),
            file_name: ff.file.name.clone(),
            file_type: ff.file.file_type.clone(),
            file_size: ff.file.size_formatted.clone(),
            source_path: ff.file.full_path.clone(),
            source_folder: ff.dir_path.clone(),
            ancestor_path: ff.ancestor_path.clone(),
            brand,
            brand_confidence,
            date: date_info,
            is_raw: p.is_raw,
            sub_bucket: p.sub.as_ref().map(|s| s.sub.clone()),
            sub_bucket_ambiguous: is_ambiguous_sub_bucket,
            template_id: p.template_id.clone(),
            project_name: p.project_name.clone(),
            destination_path: destination.full,
            destination_levels: destination.levels,
            confidence,
            needs_review,
        });
    }

    // ── Group into review clusters ──
    struct Group {
        template_id: String,
        brand: Option<Brand>,
        date: Option<DateInfo>,
        project_name: Option<String>,
        destination_root: String,
        files: Vec<FileAnalysis>,
        source_folders_order: Vec<String>,
        source_folders_set: HashSet<String>,
        confidence_sum: i64,
        needs_review_count: usize,
    }

    let mut group_order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();

    for fa in file_analyses {
        let brand_key = fa.brand.as_ref().map(|b| b.id.clone()).unwrap_or_else(|| "sin-marca".to_string());
        let date_key = match &fa.date {
            Some(d) => format!(
                "{}-{}-{}",
                d.year,
                d.month.map(|m| m.to_string()).unwrap_or_else(|| "XX".to_string()),
                d.day.map(|dd| dd.to_string()).unwrap_or_else(|| "XX".to_string())
            ),
            None => "sin-fecha".to_string(),
        };
        let proj_key = if fa.template_id == "animaciones" {
            fa.project_name.clone().unwrap_or_else(|| "proyecto".to_string())
        } else {
            String::new()
        };
        let group_key = format!("{}|{}|{}|{}", fa.template_id, brand_key, date_key, proj_key);

        if !groups.contains_key(&group_key) {
            group_order.push(group_key.clone());
            groups.insert(
                group_key.clone(),
                Group {
                    template_id: fa.template_id.clone(),
                    brand: fa.brand.clone(),
                    date: fa.date.clone(),
                    project_name: fa.project_name.clone(),
                    destination_root: fa.destination_levels.first().cloned().unwrap_or_default(),
                    files: Vec::new(),
                    source_folders_order: Vec::new(),
                    source_folders_set: HashSet::new(),
                    confidence_sum: 0,
                    needs_review_count: 0,
                },
            );
        }
        let g = groups.get_mut(&group_key).unwrap();
        if g.source_folders_set.insert(fa.source_folder.clone()) {
            g.source_folders_order.push(fa.source_folder.clone());
        }
        g.confidence_sum += fa.confidence as i64;
        if fa.needs_review {
            g.needs_review_count += 1;
        }
        g.files.push(fa);
    }

    let type_label_for = |template_id: &str| -> String {
        match template_id {
            "producciones" => "Producciones".to_string(),
            "coberturas" => "Coberturas".to_string(),
            "animaciones" => "Animaciones".to_string(),
            "recursos" => "Recursos".to_string(),
            other => templates
                .iter()
                .find(|t| t.id == other)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| other.to_string()),
        }
    };

    let mut proposals: Vec<Proposal> = Vec::new();
    for key in group_order {
        let g = groups.remove(&key).unwrap();
        let avg_confidence = (g.confidence_sum as f64 / g.files.len().max(1) as f64).round() as i32;
        let needs_review = (g.needs_review_count as f64 / g.files.len().max(1) as f64) > 0.3 || avg_confidence < 60;
        let type_label = type_label_for(&g.template_id);
        let is_resource = g.template_id == "recursos";

        let label = if is_resource {
            let file_type = g.files.first().map(|f| f.file_type.clone()).unwrap_or_else(|| "archivos".to_string());
            format!("Recursos ({})", file_type)
        } else {
            let brand_name = g.brand.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| "Sin Marca".to_string());
            let date_label = match &g.date {
                Some(d) => {
                    let day_part = d.day.map(|day| format!("{:02} ", day)).unwrap_or_default();
                    let month_part = d.month.map(month_name).unwrap_or("");
                    format!("{}{} {}", day_part, month_part, d.year).trim().to_string()
                }
                None => "Sin fecha".to_string(),
            };
            match &g.project_name {
                Some(pn) if !pn.is_empty() => format!("{} — {} {} ({})", pn, type_label, brand_name, date_label),
                _ => format!("{} {} — {}", type_label, brand_name, date_label),
            }
        };

        proposals.push(Proposal {
            id: key,
            label,
            template_id: g.template_id,
            template_name: type_label,
            brand: g.brand,
            date: g.date,
            project_name: g.project_name,
            destination_root: g.destination_root,
            file_count: g.files.len(),
            source_folder_count: g.source_folders_order.len(),
            source_folders: g.source_folders_order,
            confidence: avg_confidence,
            needs_review,
            is_resource,
            files: g.files,
            status: "pending".to_string(),
            user_path_override: None,
        });
    }

    proposals.sort_by(|a, b| {
        if a.is_resource != b.is_resource {
            if a.is_resource { Ordering::Greater } else { Ordering::Less }
        } else {
            b.confidence.cmp(&a.confidence)
        }
    });

    let mut new_brand_pairs: Vec<(String, i32)> =
        new_brand_order.into_iter().map(|k| (k.clone(), new_brand_votes[&k])).collect();
    new_brand_pairs.sort_by(|a, b| b.1.cmp(&a.1));
    let new_brands: Vec<String> = new_brand_pairs.into_iter().take(5).map(|(k, _)| k).collect();

    let summary = AnalyzeSummary {
        total_folders: crate::scanner::count_all_dirs(tree) as i64 - 1,
        total_files: all_files.len(),
        proposals: proposals.len(),
        needs_review: proposals.iter().filter(|p| p.needs_review).count(),
        high_confidence: proposals.iter().filter(|p| p.confidence >= 80).count(),
        new_brands,
        resources: proposals.iter().filter(|p| p.is_resource).count(),
    };

    logger.info(
        "analyzer",
        &format!(
            "Análisis completado: {} grupos, {} requieren revisión, {} alta confianza",
            proposals.len(),
            summary.needs_review,
            summary.high_confidence
        ),
        Some(serde_json::to_value(&summary).unwrap_or_default()),
    );

    (proposals, summary)
}
