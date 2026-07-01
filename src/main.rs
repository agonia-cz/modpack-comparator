#![windows_subsystem = "windows"]

mod lang;
mod scanner;

use eframe::egui;
use lang::{Lang, T};
use scanner::{
    build_display_name, build_file_prefix, build_timestamped_prefix, compare_snapshots,
    edition_slug, generate_markdown, scan_mods_directory, Changes, Snapshot,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

#[derive(serde::Serialize, serde::Deserialize)]
struct AppSettings {
    lang: Lang,
}

fn app_settings_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("porovnavac")
            .join("settings.json"),
    )
}

fn load_saved_language() -> Lang {
    let Some(path) = app_settings_path() else {
        return Lang::Cs;
    };

    std::fs::read_to_string(path)
        .ok()
        .and_then(|txt| serde_json::from_str::<AppSettings>(&txt).ok())
        .map(|s| s.lang)
        .unwrap_or(Lang::Cs)
}

fn save_language(lang: Lang) {
    let Some(path) = app_settings_path() else {
        return;
    };

    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }

    let settings = AppSettings { lang };
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(path, json);
    }
}

fn load_icon() -> Option<egui::IconData> {
    let icon_bytes = include_bytes!("../app-logo.ico");
    let img = image::load_from_memory(icon_bytes).ok()?.into_rgba8();
    let (w, h) = (img.width(), img.height());
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

fn main() -> eframe::Result<()> {
    let startup_lang = load_saved_language();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([760.0, 680.0])
        .with_min_inner_size([500.0, 400.0]);

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        T::window_title(startup_lang),
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new(startup_lang)))
        }),
    )
}

// ──────────────────────────────────────────────────────────────────────
// Modrinth profile detection
// ──────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ModrinthProfile {
    #[allow(dead_code)]
    folder_name: String,
    display_name: String,
    mods_path: PathBuf,
    jar_count: usize,
}

fn load_aliases() -> HashMap<String, String> {
    let appdata = match std::env::var_os("APPDATA") {
        Some(a) => PathBuf::from(a),
        None => return HashMap::new(),
    };

    let path = appdata
        .join("ModrinthApp")
        .join("profiles")
        .join("aliases.json");

    if !path.exists() {
        let defaults: HashMap<&str, &str> = [
            ("Agonia.cz (3)", "Agonia Lite"),
            ("Agonia.cz (2)", "Agonia Full"),
        ]
        .into_iter()
        .collect();
        if let Ok(json) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(&path, json);
        }
        return defaults
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
    }

    std::fs::read_to_string(&path)
        .ok()
        .and_then(|txt| serde_json::from_str(&txt).ok())
        .unwrap_or_default()
}

fn detect_modrinth_profiles() -> Vec<ModrinthProfile> {
    let mut profiles = Vec::new();
    let aliases = load_aliases();

    let appdata = match std::env::var_os("APPDATA") {
        Some(a) => PathBuf::from(a),
        None => return profiles,
    };

    let profiles_dir = appdata.join("ModrinthApp").join("profiles");
    if !profiles_dir.exists() {
        return profiles;
    }

    if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let mods_path = path.join("mods");
            if !mods_path.exists() {
                continue;
            }
            let folder_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let display_name = aliases
                .get(&folder_name)
                .cloned()
                .unwrap_or_else(|| folder_name.clone());

            let jar_count = std::fs::read_dir(&mods_path)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            let n = e.file_name().to_string_lossy().to_string();
                            n.ends_with(".jar") || n.ends_with(".jar.disabled")
                        })
                        .count()
                })
                .unwrap_or(0);

            profiles.push(ModrinthProfile {
                folder_name,
                display_name,
                mods_path,
                jar_count,
            });
        }
    }

    profiles.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    profiles
}

// ──────────────────────────────────────────────────────────────────────
// Snapshot history
// ──────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SnapshotEntry {
    filename: String,
    timestamp: String,
    path: PathBuf,
    /// Edition slug recovered from the filename (e.g. "full", "lite"), used to
    /// auto-compare a scan only against snapshots of the same edition.
    edition: Option<String>,
    snapshot: Snapshot,
}

/// Extracts the edition slug from a snapshot filename. Names look like
/// `<base>-<ver>-<edition>[-<timestamp>].mods_snapshot.json`. Older files have
/// no timestamp suffix, so the edition is the last `-` segment before the
/// extension; newer ones have the edition right before the timestamp.
fn edition_from_filename(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".mods_snapshot.json")?;
    let parts: Vec<&str> = stem.split('-').collect();
    // Timestamped: ...-<edition>-<YYYYmmdd>-<HHMMSS>
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        let prev = parts[parts.len() - 2];
        let is_ts = prev.len() == 8
            && prev.chars().all(|c| c.is_ascii_digit())
            && last.len() == 6
            && last.chars().all(|c| c.is_ascii_digit());
        if is_ts {
            return parts.get(parts.len() - 3).map(|s| s.to_string());
        }
    }
    parts.last().map(|s| s.to_string())
}

/// Human-readable edition label for a snapshot's edition slug. Capitalizes the
/// first letter; falls back to "?" when the edition couldn't be parsed.
fn pretty_edition(edition: Option<&str>) -> String {
    match edition {
        Some(e) if !e.is_empty() => {
            let mut chars = e.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "?".to_string(),
            }
        }
        _ => "?".to_string(),
    }
}

fn find_snapshot_history(profile_dir: &PathBuf) -> Vec<SnapshotEntry> {
    let mut entries = Vec::new();

    if let Ok(rd) = std::fs::read_dir(profile_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".mods_snapshot.json") {
                let Some(snapshot) = std::fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|txt| serde_json::from_str::<Snapshot>(&txt).ok())
                else {
                    continue;
                };

                entries.push(SnapshotEntry {
                    filename: name.clone(),
                    timestamp: snapshot.timestamp.clone(),
                    path: entry.path(),
                    edition: edition_from_filename(&name),
                    snapshot,
                });
            }
        }
    }

    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
}

/// Most recent snapshot of the given edition, used as the baseline for an
/// automatic comparison. Assumes `history` is sorted newest-first.
fn latest_snapshot_for_edition<'a>(
    history: &'a [SnapshotEntry],
    edition: &str,
) -> Option<&'a SnapshotEntry> {
    let want = edition_slug(edition);
    history
        .iter()
        .find(|e| e.edition.as_deref() == Some(want.as_str()))
}

// ──────────────────────────────────────────────────────────────────────
// Async scan result
// ──────────────────────────────────────────────────────────────────────

struct ScanResult {
    snapshot: Snapshot,
    old_snapshot: Option<Snapshot>,
    changes: Changes,
    markdown: String,
    snapshot_path: PathBuf,
    md_path: PathBuf,
}

// ──────────────────────────────────────────────────────────────────────
// App
// ──────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Tab {
    Settings,
    Results,
    Markdown,
    History,
}

struct App {
    lang: Lang,
    mods_dir: String,
    base_name: String,
    edition_index: usize,
    pack_version: String,
    pack_version_dirty: bool,
    force_new: bool,
    profiles: Vec<ModrinthProfile>,
    selected_profile: Option<usize>,
    scan_rx: Option<mpsc::Receiver<ScanResult>>,
    scanning: bool,
    tab: Tab,
    snapshot: Option<Snapshot>,
    old_snapshot: Option<Snapshot>,
    changes: Option<Changes>,
    markdown: String,
    status: String,
    scan_done: bool,
    history: Vec<SnapshotEntry>,
    history_selected_a: Option<usize>,
    history_selected_b: Option<usize>,
    history_changes: Option<Changes>,
    history_markdown: String,
}

const EDITIONS: [&str; 2] = ["Full", "Lite"];

impl App {
    fn new(startup_lang: Lang) -> Self {
        let profiles = detect_modrinth_profiles();
        let selected = profiles
            .iter()
            .position(|p| p.display_name.contains("Agonia"));
        let mods_dir = selected
            .map(|i| profiles[i].mods_path.to_string_lossy().to_string())
            .unwrap_or_default();
        let pack_version = selected
            .and_then(|i| read_pack_version_from_profile(&profiles[i].mods_path))
            .unwrap_or_else(|| "26.1.0".to_string());

        Self {
            lang: startup_lang,
            mods_dir,
            base_name: "Agonia".to_string(),
            edition_index: 0,
            pack_version,
            pack_version_dirty: false,
            force_new: false,
            profiles,
            selected_profile: selected,
            scan_rx: None,
            scanning: false,
            tab: Tab::Settings,
            snapshot: None,
            old_snapshot: None,
            changes: None,
            markdown: String::new(),
            status: String::new(),
            scan_done: false,
            history: Vec::new(),
            history_selected_a: None,
            history_selected_b: None,
            history_changes: None,
            history_markdown: String::new(),
        }
    }

    fn edition(&self) -> &str {
        EDITIONS[self.edition_index]
    }

    fn profile_dir(&self) -> Option<PathBuf> {
        let p = PathBuf::from(&self.mods_dir);
        p.parent().map(|p| p.to_path_buf())
    }

    fn l(&self) -> Lang {
        self.lang
    }

    fn profile_config_path(&self) -> Option<PathBuf> {
        let mods_path = PathBuf::from(&self.mods_dir);
        let profile_dir = mods_path.parent()?;
        Some(packbranding_config_path(profile_dir))
    }

    fn has_packbranding_config(&self) -> bool {
        self.profile_config_path().map(|p| p.exists()).unwrap_or(false)
    }

    fn load_pack_version_from_config(&mut self) {
        let l = self.l();
        let path = match self.profile_config_path() {
            Some(p) => p,
            None => {
                self.status = T::version_config_not_found(l).to_string();
                return;
            }
        };

        match read_pack_version_from_config(&path) {
            Ok(Some(version)) => {
                self.pack_version = version.clone();
                self.pack_version_dirty = false;
                self.status = T::version_loaded(l, &version);
            }
            Ok(None) => {
                self.status = T::version_key_missing(l).to_string();
            }
            Err(_) => {
                self.status = T::version_config_not_found(l).to_string();
            }
        }
    }

    fn save_pack_version_to_config(&mut self) {
        let l = self.l();
        let path = match self.profile_config_path() {
            Some(p) => p,
            None => {
                self.status = T::version_config_not_found(l).to_string();
                return;
            }
        };

        if !path.exists() {
            self.status = T::version_config_not_found(l).to_string();
            return;
        }

        match write_pack_version_to_config(&path, &self.pack_version) {
            Ok(()) => {
                self.pack_version_dirty = false;
                self.status = T::version_saved(l, &self.pack_version);
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    self.status = T::version_config_not_found(l).to_string();
                    return;
                }
                self.status = T::version_save_failed(l, &err.to_string());
            }
        }
    }
}

/// Resolves the PackBranding config file inside a profile's config directory.
///
/// Newer PackBranding versions use `config.json`; older ones used
/// `menu.properties`. Prefer the JSON config when present, otherwise fall back
/// to the legacy properties file (also used as the default path when neither
/// exists yet, so callers reporting "config not found" stay accurate).
fn packbranding_config_path(profile_dir: &std::path::Path) -> PathBuf {
    let dir = profile_dir.join("config").join("packbranding");
    let json = dir.join("config.json");
    if json.exists() {
        return json;
    }
    let properties = dir.join("menu.properties");
    if properties.exists() {
        return properties;
    }
    json
}

fn is_json_config(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn read_pack_version_from_profile(mods_path: &PathBuf) -> Option<String> {
    let profile_dir = mods_path.parent()?;
    let path = packbranding_config_path(profile_dir);
    read_pack_version_from_config(&path).ok().flatten()
}

fn read_pack_version_from_config(path: &PathBuf) -> std::io::Result<Option<String>> {
    if is_json_config(path) {
        read_pack_version_from_json(path)
    } else {
        read_pack_version_from_menu_properties(path)
    }
}

fn read_pack_version_from_json(path: &PathBuf) -> std::io::Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(value
        .get("packVersion")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string()))
}

fn read_pack_version_from_menu_properties(path: &PathBuf) -> std::io::Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("packVersion=") {
            return Ok(Some(value.trim().to_string()));
        }
    }
    Ok(None)
}

fn write_pack_version_to_config(path: &PathBuf, new_version: &str) -> std::io::Result<()> {
    if is_json_config(path) {
        write_pack_version_to_json(path, new_version)
    } else {
        write_pack_version_to_menu_properties(path, new_version)
    }
}

/// Updates only the `"packVersion": "..."` value in place via a line scan, so
/// the rest of the JSON (comments in `_`-prefixed keys, key order, formatting)
/// is preserved exactly.
fn write_pack_version_to_json(path: &PathBuf, new_version: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let re = regex::Regex::new(r#"("packVersion"\s*:\s*")([^"]*)(")"#).unwrap();
    if !re.is_match(&text) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "packVersion key not found",
        ));
    }
    let escaped = new_version
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let updated = re
        .replace(&text, |caps: &regex::Captures| {
            format!("{}{}{}", &caps[1], escaped, &caps[3])
        })
        .into_owned();
    std::fs::write(path, updated)
}

fn write_pack_version_to_menu_properties(path: &PathBuf, new_version: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let mut replaced = false;
    let mut out = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        if !replaced && !trimmed.starts_with('#') && trimmed.starts_with("packVersion=") {
            let indent_len = line.len() - trimmed.len();
            let indent = &line[..indent_len];
            out.push(format!("{indent}packVersion={}", new_version.trim()));
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }

    if !replaced {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "packVersion key not found",
        ));
    }

    let mut merged = out.join("\n");
    if text.contains("\r\n") {
        merged = merged.replace('\n', "\r\n");
    }
    std::fs::write(path, merged)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let l = self.l();

        if let Some(rx) = &self.scan_rx {
            if let Ok(result) = rx.try_recv() {
                if let Ok(json) = serde_json::to_string_pretty(&result.snapshot) {
                    let _ = std::fs::write(&result.snapshot_path, &json);
                }
                let _ = std::fs::write(&result.md_path, &result.markdown);

                self.status = T::scan_done(
                    l,
                    result.snapshot.stats.active,
                    result.snapshot.stats.disabled,
                    result.snapshot.stats.failed,
                    result.changes.total_changes(),
                );

                self.markdown = result.markdown;
                self.old_snapshot = result.old_snapshot;
                self.changes = Some(result.changes);
                self.snapshot = Some(result.snapshot);
                self.scan_done = true;
                self.scanning = false;
                self.scan_rx = None;
                self.tab = Tab::Results;

                if let Some(dir) = self.profile_dir() {
                    self.history = find_snapshot_history(&dir);
                }
            }
        }

        if self.scanning {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Settings, T::tab_settings(l));
                ui.selectable_value(&mut self.tab, Tab::Results, T::tab_results(l));
                ui.selectable_value(&mut self.tab, Tab::Markdown, T::tab_markdown(l));
                ui.selectable_value(&mut self.tab, Tab::History, T::tab_history(l));
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.scanning {
                    ui.spinner();
                    ui.label(T::scanning(l));
                } else if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Settings => self.show_settings(ui),
            Tab::Results => self.show_results(ui),
            Tab::Markdown => self.show_markdown(ui),
            Tab::History => self.show_history(ui),
        });
    }
}

impl App {
    fn show_settings(&mut self, ui: &mut egui::Ui) {
        let l = self.l();

        ui.heading(T::settings_heading(l));
        ui.add_space(4.0);

        // Language selector
        let old_lang = self.lang;
        ui.horizontal(|ui| {
            ui.label(T::language_label(l));
            egui::ComboBox::from_id_salt("lang_select")
                .selected_text(self.lang.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.lang, Lang::Cs, Lang::Cs.label());
                    ui.selectable_value(&mut self.lang, Lang::En, Lang::En.label());
                });
        });
        if self.lang != old_lang {
            save_language(self.lang);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
                T::window_title(self.lang).to_string(),
            ));
        }

        ui.add_space(8.0);

        if !self.profiles.is_empty() {
            ui.horizontal(|ui| {
                ui.label(T::profile_label(l));
                let current_label = self
                    .selected_profile
                    .map(|i| {
                        format!(
                            "{} ({} JARs)",
                            self.profiles[i].display_name, self.profiles[i].jar_count
                        )
                    })
                    .unwrap_or_else(|| T::custom_path(l).to_string());

                egui::ComboBox::from_id_salt("profile_select")
                    .selected_text(&current_label)
                    .show_ui(ui, |ui| {
                        for (i, profile) in self.profiles.iter().enumerate() {
                            let label =
                                format!("{} ({} JARs)", profile.display_name, profile.jar_count);
                            if ui
                                .selectable_value(&mut self.selected_profile, Some(i), &label)
                                .clicked()
                            {
                                self.mods_dir =
                                    profile.mods_path.to_string_lossy().to_string();
                                if let Some(v) = read_pack_version_from_profile(&profile.mods_path) {
                                    self.pack_version = v;
                                    self.pack_version_dirty = false;
                                }
                            }
                        }
                        if ui
                            .selectable_value(&mut self.selected_profile, None, T::custom_path(l))
                            .clicked()
                        {}
                    });
            });
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.label(T::mods_dir_label(l));
            ui.add(egui::TextEdit::singleline(&mut self.mods_dir).desired_width(400.0));
            if ui.button(T::browse(l)).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(T::browse_title(l))
                    .pick_folder()
                {
                    self.mods_dir = path.to_string_lossy().to_string();
                    self.selected_profile = None;
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(T::pack_name_label(l));
                ui.text_edit_singleline(&mut self.base_name);
                ui.end_row();

                ui.label(T::edition_label(l));
                egui::ComboBox::from_id_salt("edition")
                    .selected_text(self.edition())
                    .show_ui(ui, |ui| {
                        for (i, ed) in EDITIONS.iter().enumerate() {
                            ui.selectable_value(&mut self.edition_index, i, *ed);
                        }
                    });
                ui.end_row();

                ui.label(T::pack_version_label(l));
                let version_response = ui.text_edit_singleline(&mut self.pack_version);
                if version_response.changed() {
                    self.pack_version_dirty = true;
                }
                ui.end_row();
            });

        ui.horizontal(|ui| {
            if ui.button(T::load_pack_version(l)).clicked() {
                self.load_pack_version_from_config();
            }
            if ui.button(T::save_pack_version(l)).clicked() {
                self.save_pack_version_to_config();
            }
        });

        ui.add_space(8.0);
        ui.checkbox(&mut self.force_new, T::force_new(l));

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        let prefix = build_file_prefix(&self.base_name, self.edition(), &self.pack_version);
        let display = build_display_name(&self.base_name, self.edition(), &self.pack_version);
        ui.label(format!("{}: {}", T::name_preview(l), display));
        ui.label(format!("Snapshot: {}-<čas>.mods_snapshot.json", prefix));
        ui.label(format!("Changelog: {}-<čas>.changelog.md", prefix));

        ui.add_space(16.0);

        let mods_path = PathBuf::from(&self.mods_dir);
        let dir_exists = mods_path.exists();

        ui.add_enabled_ui(dir_exists && !self.scanning, |ui| {
            if ui
                .button(egui::RichText::new(T::scan_button(l)).size(18.0))
                .clicked()
            {
                self.start_scan();
            }
        });

        if self.scanning {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(T::scanning_in_progress(l));
            });
        }

        if !dir_exists && !self.mods_dir.is_empty() {
            ui.colored_label(egui::Color32::RED, T::dir_not_found(l));
        }

        if dir_exists && !self.has_packbranding_config() {
            ui.colored_label(egui::Color32::YELLOW, T::version_config_not_found(l));
        }
    }

    fn start_scan(&mut self) {
        if self.pack_version_dirty {
            self.save_pack_version_to_config();
        }

        let mods_path = PathBuf::from(&self.mods_dir);
        let edition = self.edition().to_string();
        let base_name = self.base_name.clone();
        let pack_version = self.pack_version.clone();
        let force_new = self.force_new;
        let lang = self.lang;

        // Pick the comparison baseline (newest existing snapshot of this
        // edition) up front, on the UI thread, so the worker just scans + diffs.
        let baseline = if force_new {
            None
        } else {
            self.profile_dir()
                .map(|dir| find_snapshot_history(&dir))
                .as_deref()
                .and_then(|h| latest_snapshot_for_edition(h, &edition).map(|e| e.snapshot.clone()))
        };

        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.status = T::scanning(self.l()).to_string();

        thread::spawn(move || {
            // Each scan writes its own timestamped files, so previous snapshots
            // are never overwritten and the history stays complete.
            let prefix = build_timestamped_prefix(&base_name, &edition, &pack_version);
            let display_name = build_display_name(&base_name, &edition, &pack_version);

            let snapshot_dir = mods_path.parent().unwrap_or(&mods_path).to_path_buf();
            let snapshot_path = snapshot_dir.join(format!("{}.mods_snapshot.json", prefix));
            let md_path = snapshot_dir.join(format!("{}.changelog.md", prefix));

            let new_snapshot = scan_mods_directory(&mods_path);

            let old_snapshot = baseline;

            let changes = if let Some(ref old) = old_snapshot {
                compare_snapshots(old, &new_snapshot)
            } else {
                Changes {
                    added: new_snapshot.active.clone(),
                    ..Changes::default()
                }
            };

            let markdown = generate_markdown(
                &display_name,
                &changes,
                &new_snapshot,
                old_snapshot.as_ref(),
                lang,
            );

            let _ = tx.send(ScanResult {
                snapshot: new_snapshot,
                old_snapshot,
                changes,
                markdown,
                snapshot_path,
                md_path,
            });
        });
    }

    fn show_results(&mut self, ui: &mut egui::Ui) {
        let l = self.l();

        if !self.scan_done {
            ui.heading(T::no_results(l));
            ui.label(T::run_scan_first(l));
            return;
        }

        let snapshot = self.snapshot.as_ref().unwrap();
        let changes = self.changes.as_ref().unwrap();

        ui.heading(T::results_heading(l));
        ui.add_space(8.0);

        egui::Grid::new("stats_grid")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label(T::total_jars(l));
                ui.strong(snapshot.stats.total.to_string());
                ui.end_row();
                ui.label(T::active(l));
                ui.strong(snapshot.stats.active.to_string());
                ui.end_row();
                ui.label(T::disabled(l));
                ui.strong(snapshot.stats.disabled.to_string());
                ui.end_row();
                ui.label(T::read_errors(l));
                ui.strong(snapshot.stats.failed.to_string());
                ui.end_row();
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.heading(T::changes_heading(l));
        ui.add_space(4.0);

        Self::show_changes_list(ui, changes, self.old_snapshot.is_some(), l);
    }

    fn show_changes_list(ui: &mut egui::Ui, changes: &Changes, has_old: bool, l: Lang) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !changes.added.is_empty() {
                ui.collapsing(T::added(l, changes.added.len()), |ui| {
                    let mut sorted = changes.added.clone();
                    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    for m in &sorted {
                        ui.label(format!("  {} v{}", m.name, m.version));
                    }
                });
            }

            if !changes.updated.is_empty() {
                ui.collapsing(T::updated(l, changes.updated.len()), |ui| {
                    let mut sorted = changes.updated.clone();
                    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    for m in &sorted {
                        ui.label(T::updated_detail(l, &m.name, &m.new_version, &m.old_version));
                    }
                });
            }

            if !changes.removed.is_empty() {
                ui.collapsing(T::removed(l, changes.removed.len()), |ui| {
                    let mut sorted = changes.removed.clone();
                    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    for m in &sorted {
                        ui.label(format!("  {} v{}", m.name, m.version));
                    }
                });
            }

            if !changes.newly_disabled.is_empty() {
                ui.collapsing(T::newly_disabled(l, changes.newly_disabled.len()), |ui| {
                    let mut sorted = changes.newly_disabled.clone();
                    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    for m in &sorted {
                        ui.label(format!("  {} v{}", m.name, m.version));
                    }
                });
            }

            if !changes.newly_enabled.is_empty() {
                ui.collapsing(T::newly_enabled(l, changes.newly_enabled.len()), |ui| {
                    let mut sorted = changes.newly_enabled.clone();
                    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    for m in &sorted {
                        ui.label(format!("  {} v{}", m.name, m.version));
                    }
                });
            }

            if changes.total_changes() == 0 && has_old {
                ui.label(T::no_changes(l));
            }

            ui.add_space(8.0);
            ui.label(T::unchanged_summary(l, changes.unchanged.len(), changes.total_changes()));
        });
    }

    fn show_markdown(&mut self, ui: &mut egui::Ui) {
        let l = self.l();

        if self.markdown.is_empty() {
            ui.heading(T::no_report(l));
            ui.label(T::run_scan_first_short(l));
            return;
        }

        ui.heading(T::generated_markdown(l));
        ui.add_space(8.0);

        if ui.button(T::copy_to_clipboard(l)).clicked() {
            ui.ctx().copy_text(self.markdown.clone());
            self.status = T::copied(l).to_string();
        }

        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.markdown.as_str())
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
        });
    }

    fn show_history(&mut self, ui: &mut egui::Ui) {
        let l = self.l();

        ui.heading(T::history_heading(l));
        ui.add_space(8.0);

        if self.history.is_empty() {
            if let Some(dir) = self.profile_dir() {
                self.history = find_snapshot_history(&dir);
            }
        }

        ui.horizontal(|ui| {
            if ui.button(T::refresh(l)).clicked() {
                self.reload_history();
            }
            // One-click shortcut for the common case: diff the two newest.
            ui.add_enabled_ui(self.history.len() >= 2, |ui| {
                if ui.button(T::compare_latest_two(l)).clicked() {
                    self.compare_indices(1, 0);
                }
            });
        });

        ui.add_space(8.0);

        if self.history.is_empty() {
            ui.label(T::no_snapshots(l));
            ui.label(T::run_scan_for_first(l));
            return;
        }

        ui.label(T::snapshots_found(l, self.history.len()));
        ui.label(T::history_pick_hint(l));
        ui.add_space(4.0);

        // Click rows to mark the two snapshots to compare. A (older) / B (newer)
        // is assigned automatically from timestamps, so order can't be wrong.
        let mut clicked: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let sel_a = self.history_selected_a;
        let sel_b = self.history_selected_b;

        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for (i, entry) in self.history.iter().enumerate() {
                    let tag = if sel_a == Some(i) {
                        "A "
                    } else if sel_b == Some(i) {
                        "B "
                    } else {
                        "   "
                    };
                    let selected = sel_a == Some(i) || sel_b == Some(i);
                    ui.horizontal(|ui| {
                        let date = &entry.timestamp[..16.min(entry.timestamp.len())];
                        let label = format!(
                            "{}[{}]  {}  ·  {} {}",
                            tag,
                            pretty_edition(entry.edition.as_deref()),
                            date.replace('T', " "),
                            entry.snapshot.stats.active,
                            T::history_active_short(l),
                        );
                        if ui.selectable_label(selected, label).clicked() {
                            clicked = Some(i);
                        }
                        if ui
                            .small_button("🗑")
                            .on_hover_text(T::delete_snapshot(l))
                            .clicked()
                        {
                            delete = Some(i);
                        }
                    });
                }
            });

        if let Some(i) = clicked {
            self.toggle_history_selection(i);
        }
        if let Some(i) = delete {
            self.delete_snapshot(i);
        }

        ui.add_space(8.0);

        let can_compare = self.history_selected_a.is_some()
            && self.history_selected_b.is_some()
            && self.history_selected_a != self.history_selected_b;

        ui.add_enabled_ui(can_compare, |ui| {
            if ui.button(T::compare_selected(l)).clicked() {
                if let (Some(a), Some(b)) = (self.history_selected_a, self.history_selected_b) {
                    self.compare_indices(a, b);
                }
            }
        });

        if let Some(ref changes) = self.history_changes.clone() {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.heading(T::history_comparison(l));

            if !self.history_markdown.is_empty() && ui.button(T::copy_history_md(l)).clicked() {
                ui.ctx().copy_text(self.history_markdown.clone());
                self.status = T::history_md_copied(l).to_string();
            }

            ui.add_space(4.0);
            Self::show_changes_list(ui, changes, true, l);
        }
    }

    fn reload_history(&mut self) {
        if let Some(dir) = self.profile_dir() {
            self.history = find_snapshot_history(&dir);
        }
        self.history_selected_a = None;
        self.history_selected_b = None;
        self.history_changes = None;
        self.history_markdown.clear();
    }

    /// Adds `i` to the comparison selection. Keeps at most two picks; the
    /// older/newer (A/B) split is derived from indices, since `history` is
    /// sorted newest-first (lower index = newer).
    fn toggle_history_selection(&mut self, i: usize) {
        if self.history_selected_a == Some(i) {
            self.history_selected_a = None;
            return;
        }
        if self.history_selected_b == Some(i) {
            self.history_selected_b = None;
            return;
        }

        match (self.history_selected_a, self.history_selected_b) {
            (None, _) if self.history_selected_b != Some(i) => self.history_selected_a = Some(i),
            (Some(_), None) => self.history_selected_b = Some(i),
            _ => {
                // Both taken: replace the older pick (A) and keep newest as the
                // sliding second slot.
                self.history_selected_a = self.history_selected_b;
                self.history_selected_b = Some(i);
            }
        }
    }

    fn delete_snapshot(&mut self, i: usize) {
        let l = self.l();
        let entry = match self.history.get(i) {
            Some(e) => e.clone(),
            None => return,
        };
        match std::fs::remove_file(&entry.path) {
            Ok(()) => {
                self.status = T::snapshot_deleted(l, &entry.filename);
                self.reload_history();
            }
            Err(err) => {
                self.status = T::snapshot_delete_failed(l, &err.to_string());
            }
        }
    }

    /// Compares two history entries. Reorders so the older snapshot is always
    /// the baseline, regardless of which row the user clicked first.
    fn compare_indices(&mut self, i: usize, j: usize) {
        let l = self.l();
        if i >= self.history.len() || j >= self.history.len() || i == j {
            return;
        }

        // history is newest-first, so the larger index is the older snapshot.
        let (old_idx, new_idx) = if i > j { (i, j) } else { (j, i) };
        let old = self.history[old_idx].snapshot.clone();
        let new = self.history[new_idx].snapshot.clone();

        let changes = compare_snapshots(&old, &new);
        let display = build_display_name(&self.base_name, self.edition(), &self.pack_version);
        let md = generate_markdown(&display, &changes, &new, Some(&old), l);

        self.status = T::history_summary(
            l,
            changes.total_changes(),
            &self.history[old_idx].filename,
            &self.history[new_idx].filename,
        );

        self.history_selected_a = Some(old_idx);
        self.history_selected_b = Some(new_idx);
        self.history_markdown = md;
        self.history_changes = Some(changes);
    }
}
