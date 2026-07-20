//! Port of src/core/smbResolver.js — macOS-only network share resolution.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::logger::Logger;

const VOLUMES_DIR: &str = "/Volumes";

pub struct ParsedSmb {
    pub server: String,
    pub share_name: String,
    pub sub_path: String,
}

pub fn is_smb_url(p: &str) -> bool {
    p.trim().to_lowercase().starts_with("smb://")
}

pub fn parse_smb_url(smb_url: &str) -> Result<ParsedSmb, String> {
    let without_proto = smb_url.trim_start_matches("smb://").trim_start_matches("SMB://").trim_start_matches("Smb://");
    // Handle case-insensitive prefix strip more robustly:
    let without_proto = if smb_url.len() >= 6 && smb_url[..6].eq_ignore_ascii_case("smb://") {
        &smb_url[6..]
    } else {
        without_proto
    };
    // Strip credentials segment (user:pass@host) if present
    let without_auth = match without_proto.find('@') {
        Some(at_idx) if !without_proto[..at_idx].contains('/') => &without_proto[at_idx + 1..],
        _ => without_proto,
    };
    let parts: Vec<&str> = without_auth.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return Err(
            "La ruta de red debe incluir el servidor y el nombre del recurso compartido. Ejemplo: smb://servidor/RecursoCompartido/carpeta"
                .to_string(),
        );
    }
    let server = urlencoding_decode(parts[0]);
    let share_name = urlencoding_decode(parts[1]);
    let sub_path = parts[2..].iter().map(|s| urlencoding_decode(s)).collect::<Vec<_>>().join("/");
    Ok(ParsedSmb { server, share_name, sub_path })
}

fn urlencoding_decode(s: &str) -> String {
    percent_decode(s)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn find_mounted_volume(share_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(VOLUMES_DIR).ok()?;
    let names: Vec<String> = entries.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect();
    if names.iter().any(|n| n == share_name) {
        return Some(Path::new(VOLUMES_DIR).join(share_name));
    }
    let lower = share_name.to_lowercase();
    let prefix_match = names.iter().find(|n| {
        let nl = n.to_lowercase();
        nl == lower || nl.starts_with(&format!("{}-", lower))
    });
    prefix_match.map(|n| Path::new(VOLUMES_DIR).join(n))
}

fn trigger_mount(smb_url: &str) {
    let _ = Command::new("open").arg(smb_url).status();
}

fn wait_for_volume(share_name: &str, timeout: Duration) -> Option<PathBuf> {
    let start = Instant::now();
    loop {
        if let Some(found) = find_mounted_volume(share_name) {
            return Some(found);
        }
        if start.elapsed() > timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// Clean up raw path input the same way normalizeLocalPath() does: strips
/// file:// URIs, percent-decodes, drops a trailing slash.
pub fn normalize_local_path(input_path: &str) -> String {
    let mut p = input_path.trim().to_string();

    if p.len() >= 7 && p[..7].eq_ignore_ascii_case("file://") {
        p = percent_decode(&p[7..]);
    } else if p.contains('%') && looks_percent_encoded(&p) {
        p = percent_decode(&p);
    }

    if p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    p
}

fn looks_percent_encoded(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() {
            return true;
        }
        i += 1;
    }
    false
}

fn recover_missing_path(p: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if !p.starts_with('/') {
        candidates.push(Path::new(VOLUMES_DIR).join(p));
    }
    if p.starts_with('/') && !p.starts_with(VOLUMES_DIR) && !p.starts_with("/Users") {
        candidates.push(Path::new(VOLUMES_DIR).join(p.trim_start_matches('/')));
    }
    if let Some(idx) = p.find("/Volumes/") {
        candidates.push(PathBuf::from(&p[idx..]));
    }
    let last_segment = p.split('/').filter(|s| !s.is_empty()).last();
    if let Some(seg) = last_segment {
        if let Ok(entries) = std::fs::read_dir(VOLUMES_DIR) {
            for e in entries.flatten() {
                let guess = e.path().join(seg);
                if guess.exists() {
                    candidates.push(guess);
                }
            }
        }
    }

    candidates.into_iter().find(|c| c.exists())
}

fn list_mounted_volumes() -> Vec<String> {
    std::fs::read_dir(VOLUMES_DIR)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|v| v != "Macintosh HD")
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPath {
    #[serde(rename = "resolvedPath")]
    pub resolved_path: String,
    #[serde(rename = "wasNetwork")]
    pub was_network: bool,
    #[serde(rename = "autoCorrected", skip_serializing_if = "Option::is_none")]
    pub auto_corrected: Option<bool>,
    #[serde(rename = "shareName", skip_serializing_if = "Option::is_none")]
    pub share_name: Option<String>,
    #[serde(rename = "mountPoint", skip_serializing_if = "Option::is_none")]
    pub mount_point: Option<String>,
}

/// Resolve any path the user gives us — local (normalizing/recovering
/// common external-drive mistakes) or an smb:// URL (mounting it first).
pub fn resolve_path(input_path: &str, logger: &Logger) -> Result<ResolvedPath, String> {
    let trimmed = normalize_local_path(input_path);

    if !is_smb_url(&trimmed) {
        if Path::new(&trimmed).exists() {
            return Ok(ResolvedPath { resolved_path: trimmed, was_network: false, auto_corrected: None, share_name: None, mount_point: None });
        }

        if let Some(recovered) = recover_missing_path(&trimmed) {
            let recovered_s = recovered.to_string_lossy().to_string();
            logger.info(
                "paths",
                "Ruta corregida automáticamente",
                Some(serde_json::json!({ "original": input_path, "resolvedPath": recovered_s })),
            );
            return Ok(ResolvedPath {
                resolved_path: recovered_s,
                was_network: false,
                auto_corrected: Some(true),
                share_name: None,
                mount_point: None,
            });
        }

        let mounted = list_mounted_volumes();
        let extra = if !mounted.is_empty() {
            format!(
                "\n\nDiscos externos conectados ahora mismo: {}. Verifica que la ruta empiece con /Volumes/<nombre del disco>/",
                mounted.join(", ")
            )
        } else {
            "\n\nNo se detecta ningún disco externo montado en este momento — confirma que esté conectado.".to_string()
        };
        return Err(format!("Carpeta no encontrada: {}{}", trimmed, extra));
    }

    let parsed = parse_smb_url(&trimmed)?;
    logger.info(
        "smb",
        &format!("Resolviendo ruta de red: {}", parsed.share_name),
        Some(serde_json::json!({ "server": parsed.server, "shareName": parsed.share_name, "subPath": parsed.sub_path })),
    );

    let mut mount_point = find_mounted_volume(&parsed.share_name);

    if mount_point.is_none() {
        logger.info("smb", &format!("Recurso \"{}\" no está montado, solicitando montaje a macOS...", parsed.share_name), None);
        trigger_mount(&trimmed);
        mount_point = wait_for_volume(&parsed.share_name, Duration::from_secs(25));
    }

    let mount_point = match mount_point {
        Some(m) => m,
        None => {
            logger.error(
                "smb",
                &format!("No se pudo montar \"{}\"", parsed.share_name),
                Some(serde_json::json!({ "server": parsed.server, "shareName": parsed.share_name })),
            );
            return Err(format!(
                "No se pudo montar \"{}\". Verifica que:\n\
                 • Estés conectado a la misma red que el NAS\n\
                 • El NAS esté encendido y accesible\n\
                 • Hayas ingresado las credenciales si Finder te las pidió\n\n\
                 También puedes montarlo manualmente desde Finder (⌘K) y luego escribir la ruta bajo /Volumes/.",
                parsed.share_name
            ));
        }
    };

    let resolved_path = if parsed.sub_path.is_empty() { mount_point.clone() } else { mount_point.join(&parsed.sub_path) };

    if !resolved_path.exists() {
        logger.error(
            "smb",
            "Subcarpeta no encontrada dentro del recurso montado",
            Some(serde_json::json!({
                "mountPoint": mount_point.to_string_lossy(), "subPath": parsed.sub_path, "resolvedPath": resolved_path.to_string_lossy()
            })),
        );
        return Err(format!(
            "El servidor \"{}\" se montó correctamente, pero no se encontró la subcarpeta:\n\"{}\"\n\nVerifica que la ruta dentro del recurso compartido sea correcta.",
            parsed.share_name, parsed.sub_path
        ));
    }

    logger.info("smb", "Ruta de red resuelta correctamente", Some(serde_json::json!({ "resolvedPath": resolved_path.to_string_lossy() })));

    Ok(ResolvedPath {
        resolved_path: resolved_path.to_string_lossy().to_string(),
        was_network: true,
        auto_corrected: None,
        share_name: Some(parsed.share_name),
        mount_point: Some(mount_point.to_string_lossy().to_string()),
    })
}
