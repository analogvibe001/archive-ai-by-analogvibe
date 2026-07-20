//! Port of src/core/brandDetector.js

use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Brand {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrandsFile {
    known: Vec<Brand>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrandMatch {
    pub brand: Brand,
    pub confidence: i32,
}

static RE_NONALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9\s]").unwrap());

pub fn normalize(s: &str) -> String {
    let mut out = s.to_lowercase();
    let pairs: [(char, char); 15] = [
        ('á', 'a'), ('à', 'a'), ('ä', 'a'),
        ('é', 'e'), ('è', 'e'), ('ë', 'e'),
        ('í', 'i'), ('ì', 'i'), ('ï', 'i'),
        ('ó', 'o'), ('ò', 'o'), ('ö', 'o'),
        ('ú', 'u'), ('ù', 'u'), ('ü', 'u'),
    ];
    for (from, to) in pairs {
        out = out.replace(from, &to.to_string());
    }
    let out = RE_NONALNUM.replace_all(&out, " ").to_string();
    out.trim().to_string()
}

pub fn tokenize(s: &str) -> Vec<String> {
    normalize(s)
        .split_whitespace()
        .filter(|t| t.len() > 1)
        .map(|t| t.to_string())
        .collect()
}

pub fn slugify_id(s: &str) -> String {
    let n = normalize(s).replace(' ', "_");
    n.trim_matches('_').to_string()
}

pub fn detect_brand(text: &str, known: &[Brand]) -> Option<BrandMatch> {
    let tokens = tokenize(text);
    let normalized = normalize(text);

    for brand in known {
        for alias in &brand.aliases {
            let norm_alias = normalize(alias);
            if norm_alias.is_empty() {
                continue;
            }
            if normalized.contains(&norm_alias) {
                let confidence = if norm_alias == normalized { 98 } else { 90 };
                return Some(BrandMatch { brand: brand.clone(), confidence });
            }
            if tokens.iter().any(|t| *t == norm_alias || (norm_alias.starts_with(t.as_str()) && t.len() >= 3)) {
                return Some(BrandMatch { brand: brand.clone(), confidence: 75 });
            }
        }
    }
    None
}

/// Detect brand by scanning an ancestor folder chain (outermost first).
/// Outer folders weigh more than inner ones.
pub fn detect_from_chain(chain: &[String], known: &[Brand]) -> Option<BrandMatch> {
    struct Vote {
        brand: Brand,
        score: f64,
        count: i32,
    }
    let mut votes: HashMap<String, Vote> = HashMap::new();
    let len = chain.len();
    for (idx, folder_name) in chain.iter().enumerate() {
        if let Some(result) = detect_brand(folder_name, known) {
            let depth_weight = std::cmp::max(1, len as i64 - idx as i64) as f64;
            let entry = votes.entry(result.brand.id.clone()).or_insert(Vote {
                brand: result.brand.clone(),
                score: 0.0,
                count: 0,
            });
            entry.score += result.confidence as f64 * depth_weight;
            entry.count += 1;
        }
    }
    let mut sorted: Vec<&Vote> = votes.values().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    let best = sorted.first()?;
    let conf = (best.score / (best.count as f64 * len as f64)).round().min(99.0);
    Some(BrandMatch { brand: best.brand.clone(), confidence: (conf as i32).max(40) })
}

pub struct Suggestion {
    pub suggested: String,
    pub token: String,
}

static IGNORE_TOKENS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "fotos", "videos", "audio", "enero", "febrero", "marzo", "abril", "mayo",
        "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
        "lunes", "martes", "miercoles", "jueves", "viernes", "sabado", "domingo",
        "produccion", "animacion", "cobertura", "recurso", "proyecto", "carpeta",
        "assets", "exports", "editable", "raw", "edit", "final", "backup", "temp",
        "2020", "2021", "2022", "2023", "2024", "2025", "2026", "2027",
    ]
    .into_iter()
    .collect()
});

pub fn suggest_new_brand(name: &str, known: &[Brand]) -> Option<Suggestion> {
    let known_aliases: HashSet<String> = known.iter().flat_map(|b| b.aliases.iter().map(|a| normalize(a))).collect();
    let tokens = tokenize(name);
    for t in tokens {
        if t.len() < 3 {
            continue;
        }
        if IGNORE_TOKENS.contains(t.as_str()) {
            continue;
        }
        if known_aliases.contains(&t) {
            continue;
        }
        if t.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let mut chars = t.chars();
        let suggested = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => continue,
        };
        return Some(Suggestion { suggested, token: t });
    }
    None
}

// ── Store: load/persist data/brands.json ─────────────────────────────
pub struct BrandStore {
    path: PathBuf,
    cache: Mutex<Option<Vec<Brand>>>,
}

impl BrandStore {
    pub fn new(path: PathBuf) -> Self {
        BrandStore { path, cache: Mutex::new(None) }
    }

    pub fn load(&self) -> Vec<Brand> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(known) = &*cache {
            return known.clone();
        }
        let known = fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str::<BrandsFile>(&s).ok())
            .map(|f| f.known)
            .unwrap_or_default();
        *cache = Some(known.clone());
        known
    }

    fn persist(&self, known: Vec<Brand>) -> Result<(), String> {
        let file = BrandsFile { known: known.clone() };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&self.path, json).map_err(|e| e.to_string())?;
        *self.cache.lock().unwrap() = Some(known);
        Ok(())
    }

    pub fn add(&self, input: BrandInput) -> Result<Brand, String> {
        let mut known = self.load();
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err("El nombre de la marca es obligatorio".to_string());
        }
        let id_source = input.id.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| name.clone());
        let id = slugify_id(&id_source);
        if id.is_empty() {
            return Err("No se pudo generar un identificador para la marca".to_string());
        }
        if known.iter().any(|b| b.id == id) {
            return Err(format!("Ya existe una marca con el id \"{}\"", id));
        }
        let aliases = if !input.aliases.is_empty() {
            input.aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect()
        } else {
            vec![name.to_lowercase()]
        };
        let color = input
            .color
            .filter(|c| Regex::new(r"(?i)^#[0-9a-f]{6}$").unwrap().is_match(c))
            .unwrap_or_else(random_color);

        let brand = Brand { id, name, aliases, color };
        known.push(brand.clone());
        self.persist(known)?;
        Ok(brand)
    }

    pub fn update(&self, id: &str, patch: BrandPatch) -> Result<Brand, String> {
        let mut known = self.load();
        let idx = known.iter().position(|b| b.id == id).ok_or_else(|| format!("Marca \"{}\" no encontrada", id))?;
        {
            let b = &mut known[idx];
            if let Some(name) = &patch.name {
                if !name.trim().is_empty() {
                    b.name = name.trim().to_string();
                }
            }
            if let Some(color) = &patch.color {
                if Regex::new(r"(?i)^#[0-9a-f]{6}$").unwrap().is_match(color) {
                    b.color = color.clone();
                }
            }
            if let Some(aliases) = &patch.aliases {
                let mut a: Vec<String> = aliases.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
                if a.is_empty() {
                    a = vec![b.name.to_lowercase()];
                }
                b.aliases = a;
            }
        }
        let updated = known[idx].clone();
        self.persist(known)?;
        Ok(updated)
    }

    pub fn remove(&self, id: &str) -> Result<Brand, String> {
        let mut known = self.load();
        let idx = known.iter().position(|b| b.id == id).ok_or_else(|| format!("Marca \"{}\" no encontrada", id))?;
        let removed = known.remove(idx);
        self.persist(known)?;
        Ok(removed)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct BrandInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub color: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BrandPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub aliases: Option<Vec<String>>,
}

fn random_color() -> String {
    let mut rng = rand::thread_rng();
    let v: u32 = rng.gen_range(0..0xffffff);
    format!("#{:06x}", v)
}
