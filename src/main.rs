#![windows_subsystem = "windows"]

mod lang;
mod scanner;

use eframe::egui;
use lang::{Lang, T};
use scanner::{
    build_display_name, build_file_prefix, compare_snapshots, generate_markdown,
    scan_mods_directory, Changes, Snapshot,
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
}

fn find_snapshot_history(profile_dir: &PathBuf) -> Vec<SnapshotEntry> {
    let mut entries = Vec::new();

    if let Ok(rd) = std::fs::read_dir(profile_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".mods_snapshot.json") {
                let timestamp = std::fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|txt| serde_json::from_str::<Snapshot>(&txt).ok())
                    .map(|s| s.timestamp)
                    .unwrap_or_else(|| "?".to_string());

                entries.push(SnapshotEntry {
                    filename: name,
                    timestamp,
                    path: entry.path(),
                });
            }
        }
    }

    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
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
        Some(profile_dir.join("config").join("packbranding").join("menu.properties"))
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

        match read_pack_version_from_menu_properties(&path) {
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

        match write_pack_version_to_menu_properties(&path, &self.pack_version) {
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

fn read_pack_version_from_profile(mods_path: &PathBuf) -> Option<String> {
    let profile_dir = mods_path.parent()?;
    let path = profile_dir
        .join("config")
        .join("packbranding")
        .join("menu.properties");
    read_pack_version_from_menu_properties(&path).ok().flatten()
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
        ui.label(format!("Snapshot: {}.mods_snapshot.json", prefix));
        ui.label(format!("Changelog: {}.changelog.md", prefix));

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

        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.status = T::scanning(self.l()).to_string();

        thread::spawn(move || {
            let prefix = build_file_prefix(&base_name, &edition, &pack_version);
            let display_name = build_display_name(&base_name, &edition, &pack_version);

            let snapshot_dir = mods_path.parent().unwrap_or(&mods_path).to_path_buf();
            let snapshot_path = snapshot_dir.join(format!("{}.mods_snapshot.json", prefix));
            let md_path = snapshot_dir.join(format!("{}.changelog.md", prefix));

            let new_snapshot = scan_mods_directory(&mods_path);

            let old_snapshot = if snapshot_path.exists() && !force_new {
                std::fs::read_to_string(&snapshot_path)
                    .ok()
                    .and_then(|txt| serde_json::from_str::<Snapshot>(&txt).ok())
            } else {
                None
            };

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

        if ui.button(T::refresh(l)).clicked() {
            if let Some(dir) = self.profile_dir() {
                self.history = find_snapshot_history(&dir);
                self.history_selected_a = None;
                self.history_selected_b = None;
                self.history_changes = None;
                self.history_markdown.clear();
            }
        }

        if self.history.is_empty() {
            if let Some(dir) = self.profile_dir() {
                self.history = find_snapshot_history(&dir);
            }
        }

        ui.add_space(8.0);

        if self.history.is_empty() {
            ui.label(T::no_snapshots(l));
            ui.label(T::run_scan_for_first(l));
            return;
        }

        ui.label(T::snapshots_found(l, self.history.len()));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(T::older_snapshot(l));
                egui::ComboBox::from_id_salt("history_a")
                    .selected_text(
                        self.history_selected_a
                            .map(|i| self.history[i].filename.clone())
                            .unwrap_or_else(|| T::select(l).to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, entry) in self.history.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.history_selected_a,
                                Some(i),
                                format!(
                                    "{} ({})",
                                    entry.filename,
                                    &entry.timestamp[..10.min(entry.timestamp.len())]
                                ),
                            );
                        }
                    });
            });

            ui.add_space(16.0);

            ui.vertical(|ui| {
                ui.label(T::newer_snapshot(l));
                egui::ComboBox::from_id_salt("history_b")
                    .selected_text(
                        self.history_selected_b
                            .map(|i| self.history[i].filename.clone())
                            .unwrap_or_else(|| T::select(l).to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, entry) in self.history.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.history_selected_b,
                                Some(i),
                                format!(
                                    "{} ({})",
                                    entry.filename,
                                    &entry.timestamp[..10.min(entry.timestamp.len())]
                                ),
                            );
                        }
                    });
            });
        });

        ui.add_space(8.0);

        let can_compare = self.history_selected_a.is_some()
            && self.history_selected_b.is_some()
            && self.history_selected_a != self.history_selected_b;

        ui.add_enabled_ui(can_compare, |ui| {
            if ui.button(T::compare_selected(l)).clicked() {
                self.compare_history();
            }
        });

        if self.history_selected_a.is_some()
            && self.history_selected_b.is_some()
            && self.history_selected_a == self.history_selected_b
        {
            ui.colored_label(egui::Color32::YELLOW, T::select_two_different(l));
        }

        if let Some(ref changes) = self.history_changes.clone() {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.heading(T::history_comparison(l));

            if !self.history_markdown.is_empty() {
                if ui.button(T::copy_history_md(l)).clicked() {
                    ui.ctx().copy_text(self.history_markdown.clone());
                    self.status = T::history_md_copied(l).to_string();
                }
            }

            ui.add_space(4.0);
            Self::show_changes_list(ui, changes, true, l);
        }
    }

    fn compare_history(&mut self) {
        let l = self.l();

        let idx_a = match self.history_selected_a {
            Some(i) => i,
            None => return,
        };
        let idx_b = match self.history_selected_b {
            Some(i) => i,
            None => return,
        };

        let load = |idx: usize| -> Option<Snapshot> {
            let path = &self.history[idx].path;
            std::fs::read_to_string(path)
                .ok()
                .and_then(|txt| serde_json::from_str(&txt).ok())
        };

        let old = match load(idx_a) {
            Some(s) => s,
            None => {
                self.status = T::snapshot_read_error(l, &self.history[idx_a].filename);
                return;
            }
        };
        let new = match load(idx_b) {
            Some(s) => s,
            None => {
                self.status = T::snapshot_read_error(l, &self.history[idx_b].filename);
                return;
            }
        };

        let changes = compare_snapshots(&old, &new);
        let display = build_display_name(&self.base_name, self.edition(), &self.pack_version);
        let md = generate_markdown(&display, &changes, &new, Some(&old), l);

        self.status = T::history_summary(
            l,
            changes.total_changes(),
            &self.history[idx_a].filename,
            &self.history[idx_b].filename,
        );

        self.history_markdown = md;
        self.history_changes = Some(changes);
    }
}
