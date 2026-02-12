use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModData {
    pub filename: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub loader: String,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotStats {
    pub total: usize,
    pub active: usize,
    pub disabled: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub timestamp: String,
    pub mods_dir: String,
    pub active: Vec<ModData>,
    pub disabled: Vec<ModData>,
    pub failed: Vec<String>,
    pub stats: SnapshotStats,
}

/// Sanitize JSON text: strip BOM, control chars, comments, trailing commas.
fn sanitize_json(txt: &str) -> String {
    let txt = txt.trim_start_matches('\u{feff}');

    // Strip control chars except \t \n \r
    let control = Regex::new(r"[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]").unwrap();
    let txt = control.replace_all(txt, "");

    // Strip line comments
    let line_comments = Regex::new(r"//[^\n\r]*").unwrap();
    let txt = line_comments.replace_all(&txt, "");

    // Strip block comments
    let block_comments = Regex::new(r"(?s)/\*.*?\*/").unwrap();
    let txt = block_comments.replace_all(&txt, "");

    // Strip trailing commas
    let trailing = Regex::new(r",\s*([}\]])").unwrap();
    let txt = trailing.replace_all(&txt, "$1");

    // Escape raw newlines inside strings
    escape_raw_newlines(&txt)
}

fn escape_raw_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    let mut esc = false;

    for ch in s.chars() {
        if inside {
            if esc {
                out.push(ch);
                esc = false;
            } else if ch == '\\' {
                out.push(ch);
                esc = true;
            } else if ch == '"' {
                out.push(ch);
                inside = false;
            } else if ch == '\n' {
                out.push_str("\\n");
            } else if ch == '\r' {
                // skip
            } else {
                out.push(ch);
            }
        } else {
            if ch == '"' {
                out.push(ch);
                inside = true;
                esc = false;
            } else {
                out.push(ch);
            }
        }
    }
    out
}

/// Regex fallback: grab id, name, version from malformed JSON.
fn regex_fallback(txt: &str) -> Option<(String, String, String)> {
    let grab = |key: &str| -> Option<String> {
        let re = Regex::new(&format!(r#""{key}"\s*:\s*"([^"]+)""#)).ok()?;
        re.captures(txt).map(|c| c[1].to_string())
    };

    let id = grab("id");
    let name = grab("name");
    let version = grab("version");

    if id.is_some() || name.is_some() || version.is_some() {
        Some((
            id.unwrap_or_default(),
            name.unwrap_or_default(),
            version.unwrap_or_default(),
        ))
    } else {
        None
    }
}

/// Read mod metadata from a JAR file (fabric.mod.json or quilt.mod.json).
fn read_meta_from_jar(jar_path: &Path) -> Option<(String, String, String, String)> {
    let file = std::fs::File::open(jar_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    for candidate in &["fabric.mod.json", "quilt.mod.json"] {
        let mut entry = match archive.by_name(candidate) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut raw = Vec::new();
        if entry.read_to_end(&mut raw).is_err() {
            continue;
        }

        let txt = String::from_utf8(raw)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

        let clean = sanitize_json(&txt);

        // Try full JSON parse
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&clean) {
            let id = val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let version = val
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let loader = if val.get("depends").is_some()
                && val["depends"].get("quilt_loader").is_some()
            {
                "quilt"
            } else {
                "fabric"
            };

            return Some((id, name, version, loader.to_string()));
        }

        // Regex fallback
        if let Some((id, name, version)) = regex_fallback(&clean) {
            return Some((id, name, version, "fabric".to_string()));
        }
    }

    None
}

/// Scan all .jar and .jar.disabled files in a directory.
pub fn scan_mods_directory(mods_dir: &Path) -> Snapshot {
    let mut all_files: Vec<PathBuf> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(mods_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.ends_with(".jar") || name.ends_with(".jar.disabled") {
                all_files.push(path);
            }
        }
    }

    all_files.sort();

    let mut active = Vec::new();
    let mut disabled = Vec::new();
    let mut failed = Vec::new();

    for jar in &all_files {
        let filename = jar.file_name().unwrap_or_default().to_string_lossy().to_string();
        let is_disabled = filename.ends_with(".jar.disabled");

        match read_meta_from_jar(jar) {
            Some((id, name, version, loader)) => {
                let stem = jar.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let mod_data = ModData {
                    filename: filename.clone(),
                    id: if id.is_empty() { stem.clone() } else { id },
                    name: if name.is_empty() { stem } else { name },
                    version: if version.is_empty() {
                        "unknown".to_string()
                    } else {
                        version
                    },
                    loader,
                    disabled: is_disabled,
                };

                if is_disabled {
                    disabled.push(mod_data);
                } else {
                    active.push(mod_data);
                }
            }
            None => {
                failed.push(filename);
            }
        }
    }

    let stats = SnapshotStats {
        total: all_files.len(),
        active: active.len(),
        disabled: disabled.len(),
        failed: failed.len(),
    };

    Snapshot {
        timestamp: chrono::Local::now().to_rfc3339(),
        mods_dir: mods_dir.to_string_lossy().to_string(),
        active,
        disabled,
        failed,
        stats,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Comparison
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct UpdatedMod {
    pub id: String,
    pub name: String,
    pub old_version: String,
    pub new_version: String,
    pub filename: String,
}

#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub added: Vec<ModData>,
    pub removed: Vec<ModData>,
    pub updated: Vec<UpdatedMod>,
    pub newly_disabled: Vec<ModData>,
    pub newly_enabled: Vec<ModData>,
    pub unchanged: Vec<ModData>,
}

impl Changes {
    pub fn total_changes(&self) -> usize {
        self.added.len()
            + self.removed.len()
            + self.updated.len()
            + self.newly_disabled.len()
            + self.newly_enabled.len()
    }
}

pub fn compare_snapshots(old: &Snapshot, new: &Snapshot) -> Changes {
    let old_active: HashMap<&str, &ModData> = old.active.iter().map(|m| (m.id.as_str(), m)).collect();
    let new_active: HashMap<&str, &ModData> = new.active.iter().map(|m| (m.id.as_str(), m)).collect();
    let old_disabled: HashMap<&str, &ModData> =
        old.disabled.iter().map(|m| (m.id.as_str(), m)).collect();
    let new_disabled: HashMap<&str, &ModData> =
        new.disabled.iter().map(|m| (m.id.as_str(), m)).collect();

    let mut changes = Changes::default();

    for (mod_id, m) in &new_active {
        if !old_active.contains_key(mod_id) && !old_disabled.contains_key(mod_id) {
            changes.added.push((*m).clone());
        } else if old_disabled.contains_key(mod_id) {
            changes.newly_enabled.push((*m).clone());
        } else if let Some(old_mod) = old_active.get(mod_id) {
            if m.version != old_mod.version {
                changes.updated.push(UpdatedMod {
                    id: mod_id.to_string(),
                    name: m.name.clone(),
                    old_version: old_mod.version.clone(),
                    new_version: m.version.clone(),
                    filename: m.filename.clone(),
                });
            } else {
                changes.unchanged.push((*m).clone());
            }
        }
    }

    for (mod_id, m) in &old_active {
        if !new_active.contains_key(mod_id) && !new_disabled.contains_key(mod_id) {
            changes.removed.push((*m).clone());
        } else if let Some(dis) = new_disabled.get(mod_id) {
            changes.newly_disabled.push((*dis).clone());
        }
    }

    changes
}

// ──────────────────────────────────────────────────────────────────────
// Markdown generation
// ──────────────────────────────────────────────────────────────────────

pub fn generate_markdown(
    display_name: &str,
    changes: &Changes,
    new_snapshot: &Snapshot,
    old_snapshot: Option<&Snapshot>,
) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# 🛠️ {} — Přehled změn\n", display_name));
    lines.push(format!(
        "**Datum:** {}\n",
        chrono::Local::now().format("%d.%m.%Y %H:%M")
    ));

    let stats = &new_snapshot.stats;
    lines.push(format!(
        "**Celkem modů:** {}  •  Vypnuté: {}  •  Chyby čtení: {}\n",
        stats.active, stats.disabled, stats.failed
    ));

    if let Some(old) = old_snapshot {
        lines.push(format!("**Porovnáno s:** {}\n", old.timestamp));
    }

    lines.push("\n---\n".to_string());

    if !changes.added.is_empty() {
        lines.push(format!("## ✨ Nové módy ({})", changes.added.len()));
        let mut sorted = changes.added.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!("* `{}` v{}", m.name, m.version));
        }
        lines.push(String::new());
    }

    if !changes.updated.is_empty() {
        lines.push(format!(
            "## 🔄 Aktualizované módy ({})",
            changes.updated.len()
        ));
        let mut sorted = changes.updated.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!(
                "* `{}` → **{}** (předtím {})",
                m.name, m.new_version, m.old_version
            ));
        }
        lines.push(String::new());
    }

    if !changes.removed.is_empty() {
        lines.push(format!(
            "## ❌ Odstraněné módy ({})",
            changes.removed.len()
        ));
        let mut sorted = changes.removed.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!("* `{}` v{}", m.name, m.version));
        }
        lines.push(String::new());
    }

    if !changes.newly_disabled.is_empty() {
        lines.push(format!(
            "## 🚫 Nově vypnuté módy ({})",
            changes.newly_disabled.len()
        ));
        lines.push(
            "*Důvod: Pravděpodobně nekompatibilní nebo konfliktní s aktuální verzí*\n".to_string(),
        );
        let mut sorted = changes.newly_disabled.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!("* `{}` v{}", m.name, m.version));
        }
        lines.push(String::new());
    }

    if !changes.newly_enabled.is_empty() {
        lines.push(format!(
            "## ✅ Nově zapnuté módy ({})",
            changes.newly_enabled.len()
        ));
        let mut sorted = changes.newly_enabled.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!("* `{}` v{}", m.name, m.version));
        }
        lines.push(String::new());
    }

    if !new_snapshot.disabled.is_empty() {
        lines.push("---\n".to_string());
        lines.push(format!(
            "## 📋 Aktuálně vypnuté módy ({})",
            new_snapshot.disabled.len()
        ));
        let mut sorted = new_snapshot.disabled.clone();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for m in &sorted {
            lines.push(format!("* `{}` v{}", m.name, m.version));
        }
        lines.push(String::new());
    }

    if !new_snapshot.failed.is_empty() {
        lines.push("---\n".to_string());
        lines.push(format!(
            "## ⚠️ Soubory s chybou čtení ({})",
            new_snapshot.failed.len()
        ));
        let mut sorted = new_snapshot.failed.clone();
        sorted.sort();
        for f in &sorted {
            lines.push(format!("* `{}` — nelze přečíst metadata", f));
        }
        lines.push(String::new());
    }

    lines.push("---\n".to_string());
    lines.push("🎮 **Doporučení:** Po větších updatech může pomoct smazat `config/` (nebo aspoň konkrétní configy problematických modů).\n".to_string());
    lines.push(format!(
        "_(Beze změny: {} • Celkem změn: {})_\n",
        changes.unchanged.len(),
        changes.total_changes()
    ));

    lines.join("\n")
}

// ──────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────

pub fn slugify(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = Regex::new(r"\s+").unwrap().replace_all(&s, "-");
    let s = Regex::new(r"[^a-z0-9._\-]").unwrap().replace_all(&s, "");
    let s = Regex::new(r"-{2,}").unwrap().replace_all(&s, "-");
    if s.is_empty() {
        "pack".to_string()
    } else {
        s.to_string()
    }
}

pub fn normalize_edition(edition: &str) -> &str {
    let e = edition.trim();
    if e.is_empty() {
        return "Full";
    }
    match e.to_lowercase().as_str() {
        "full" | "normal" | "default" => "Full",
        "lite" | "light" | "minimal" => "Lite",
        _ => edition,
    }
}

pub fn build_display_name(base_name: &str, edition: &str, pack_version: &str) -> String {
    let base = if base_name.trim().is_empty() {
        "Agonia"
    } else {
        base_name.trim()
    };
    let ed = normalize_edition(edition);
    let ver = pack_version.trim();

    if ed.to_lowercase() == "full" {
        format!("{} {}", base, ver).trim().to_string()
    } else {
        format!("{} {} {}", base, ed, ver).trim().to_string()
    }
}

pub fn build_file_prefix(base_name: &str, edition: &str, pack_version: &str) -> String {
    let base = slugify(base_name);
    let ed = slugify(normalize_edition(edition));
    let ver = if pack_version.trim().is_empty() {
        "unknown".to_string()
    } else {
        slugify(pack_version)
    };
    format!("{}-{}-{}", base, ver, ed)
}
