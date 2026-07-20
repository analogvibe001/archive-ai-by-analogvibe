//! Port of src/core/templateEngine.js
//!
//! Note: `buildDestination` and `detectTemplate` from the original file are
//! not ported — grepping server.js and projectAnalyzer.js confirms neither
//! is ever called; the live code path only uses load/get/all/fillPath/
//! reload/add/update/delete. Porting dead code would add risk with no
//! behavioral benefit.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use unicode_normalization::UnicodeNormalization;

pub static BUILTIN_IDS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| ["producciones", "coberturas", "animaciones", "recursos"].into_iter().collect());

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Signals {
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub signals: Signals,
    #[serde(default)]
    pub levels: Vec<String>,
    #[serde(rename = "splitByMedia", default)]
    pub split_by_media: bool,
    #[serde(rename = "splitRawEdited", default)]
    pub split_raw_edited: bool,
    #[serde(default)]
    pub notes: String,
}

/// fillPath(pattern, vars) — replaces every {key} token; leaves unknown
/// tokens as the literal `{key}` text, exactly like the JS version.
static RE_TOKEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{([^}]+)\}").unwrap());

pub fn fill_path(pattern: &str, vars: &HashMap<String, String>) -> String {
    RE_TOKEN
        .replace_all(pattern, |caps: &regex::Captures| {
            let key = &caps[1];
            match vars.get(key) {
                Some(v) if !v.is_empty() => v.clone(),
                _ => format!("{{{}}}", key),
            }
        })
        .to_string()
}

/// slugify() as used by templateEngine.js — full Unicode NFD normalization
/// + diacritic stripping (different from brandDetector's simpler `normalize`).
pub fn slugify(s: &str) -> String {
    let decomposed: String = s.nfd().filter(|c| !is_combining_mark(*c)).collect();
    let lower = decomposed.to_lowercase();
    let re_nonalnum = Regex::new(r"[^a-z0-9]+").unwrap();
    let dashed = re_nonalnum.replace_all(&lower, "-").to_string();
    dashed.trim_matches('-').to_string()
}

fn is_combining_mark(c: char) -> bool {
    ('\u{0300}'..='\u{036f}').contains(&c)
}

fn normalize_custom_template(input: &serde_json::Value, forced_id: &str) -> Result<Template, String> {
    let levels: Vec<String> = input
        .get("levels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if levels.is_empty() {
        return Err("La jerarquía necesita al menos un nivel de carpeta".to_string());
    }

    let keywords: Vec<String> = match input.get("keywords") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_json::Value::String(s)) => s
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };

    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| forced_id.to_string());

    let split_by_media = input.get("splitByMedia").and_then(|v| v.as_bool()).unwrap_or(false);
    let split_raw_edited = split_by_media && input.get("splitRawEdited").and_then(|v| v.as_bool()).unwrap_or(false);

    Ok(Template {
        id: forced_id.to_string(),
        name,
        description: input.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        icon: Some(input.get("icon").and_then(|v| v.as_str()).unwrap_or("📁").to_string()),
        custom: true,
        signals: Signals { keywords },
        levels,
        split_by_media,
        split_raw_edited,
        notes: input.get("notes").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    })
}

pub struct TemplateStore {
    dir: PathBuf,
    cache: Mutex<Option<Vec<Template>>>,
}

impl TemplateStore {
    pub fn new(dir: PathBuf) -> Self {
        TemplateStore { dir, cache: Mutex::new(None) }
    }

    pub fn load(&self) -> Vec<Template> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(templates) = &*cache {
            return templates.clone();
        }
        let mut templates = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            let mut paths: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
                .collect();
            paths.sort();
            for path in paths {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(tpl) = serde_json::from_str::<Template>(&content) {
                        templates.push(tpl);
                    }
                }
            }
        }
        *cache = Some(templates.clone());
        templates
    }

    pub fn get(&self, id: &str) -> Option<Template> {
        self.load().into_iter().find(|t| t.id == id)
    }

    pub fn all(&self) -> Vec<Template> {
        self.load()
    }

    pub fn reload(&self) {
        *self.cache.lock().unwrap() = None;
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        BUILTIN_IDS.contains(id)
    }

    fn save_file(&self, tpl: &Template) -> Result<(), String> {
        let json = serde_json::to_string_pretty(tpl).map_err(|e| e.to_string())?;
        let _ = fs::create_dir_all(&self.dir);
        fs::write(self.dir.join(format!("{}.json", tpl.id)), json).map_err(|e| e.to_string())
    }

    pub fn add_template(&self, input: serde_json::Value) -> Result<Template, String> {
        let templates = self.load();
        let id_source = input
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| input.get("name").and_then(|v| v.as_str()))
            .unwrap_or("");
        let id = slugify(id_source);
        if id.is_empty() {
            return Err("Nombre de jerarquía inválido".to_string());
        }
        if templates.iter().any(|t| t.id == id) {
            return Err(format!("Ya existe una jerarquía con el id \"{}\"", id));
        }
        let tpl = normalize_custom_template(&input, &id)?;
        self.save_file(&tpl)?;
        let mut cache = self.cache.lock().unwrap();
        let mut list = cache.take().unwrap_or_default();
        list.push(tpl.clone());
        *cache = Some(list);
        Ok(tpl)
    }

    pub fn update_template(&self, id: &str, input: serde_json::Value) -> Result<Template, String> {
        if self.is_builtin(id) {
            return Err("No se pueden editar las jerarquías predeterminadas".to_string());
        }
        let templates = self.load();
        let existing = templates.iter().find(|t| t.id == id).ok_or_else(|| format!("Jerarquía \"{}\" no encontrada", id))?;

        let mut merged = serde_json::to_value(existing).map_err(|e| e.to_string())?;
        if let (serde_json::Value::Object(base), serde_json::Value::Object(patch)) = (&mut merged, &input) {
            for (k, v) in patch {
                base.insert(k.clone(), v.clone());
            }
        }

        let tpl = normalize_custom_template(&merged, id)?;
        self.save_file(&tpl)?;
        let mut cache = self.cache.lock().unwrap();
        let mut list = cache.take().unwrap_or_default();
        if let Some(slot) = list.iter_mut().find(|t| t.id == id) {
            *slot = tpl.clone();
        }
        *cache = Some(list);
        Ok(tpl)
    }

    pub fn delete_template(&self, id: &str) -> Result<(), String> {
        if self.is_builtin(id) {
            return Err("No se pueden eliminar las jerarquías predeterminadas".to_string());
        }
        let templates = self.load();
        if !templates.iter().any(|t| t.id == id) {
            return Err(format!("Jerarquía \"{}\" no encontrada", id));
        }
        let file_path = self.dir.join(format!("{}.json", id));
        if fs::remove_file(&file_path).is_err() {
            let _ = fs::rename(&file_path, self.dir.join(format!("{}.deleted", id)));
        }
        let mut cache = self.cache.lock().unwrap();
        let mut list = cache.take().unwrap_or_default();
        list.retain(|t| t.id != id);
        *cache = Some(list);
        Ok(())
    }
}
