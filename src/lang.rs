use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    Cs,
    En,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::Cs => "Cestina",
            Lang::En => "English",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Lang::Cs => "cs",
            Lang::En => "en",
        }
    }
}

type Translations = HashMap<String, HashMap<String, String>>;

static LANG_DATA: LazyLock<Translations> = LazyLock::new(|| {
    let raw = include_str!("lang.json");
    let val: Value = serde_json::from_str(raw).expect("lang.json is invalid");
    let mut map = Translations::new();
    if let Value::Object(langs) = val {
        for (lang_key, entries) in langs {
            let mut inner = HashMap::new();
            if let Value::Object(kv) = entries {
                for (k, v) in kv {
                    if let Value::String(s) = v {
                        inner.insert(k, s);
                    }
                }
            }
            map.insert(lang_key, inner);
        }
    }
    map
});

fn get(l: Lang, key: &str) -> &'static str {
    LANG_DATA
        .get(l.key())
        .and_then(|m| m.get(key))
        .map(|s| s.as_str())
        .unwrap_or("???")
}

fn get_owned(l: Lang, key: &str) -> String {
    get(l, key).to_string()
}

fn fmt(l: Lang, key: &str, replacements: &[(&str, &str)]) -> String {
    let mut s = get_owned(l, key);
    for (placeholder, value) in replacements {
        s = s.replace(placeholder, value);
    }
    s
}

pub struct T;

impl T {
    // ── Simple static strings ───────────────────────────────────────
    pub fn window_title(l: Lang) -> &'static str { get(l, "window_title") }
    pub fn tab_settings(l: Lang) -> &'static str { get(l, "tab_settings") }
    pub fn tab_results(l: Lang) -> &'static str { get(l, "tab_results") }
    pub fn tab_markdown(l: Lang) -> &'static str { get(l, "tab_markdown") }
    pub fn tab_history(l: Lang) -> &'static str { get(l, "tab_history") }
    pub fn scanning(l: Lang) -> &'static str { get(l, "scanning") }
    pub fn settings_heading(l: Lang) -> &'static str { get(l, "settings_heading") }
    pub fn profile_label(l: Lang) -> &'static str { get(l, "profile_label") }
    pub fn custom_path(l: Lang) -> &'static str { get(l, "custom_path") }
    pub fn mods_dir_label(l: Lang) -> &'static str { get(l, "mods_dir_label") }
    pub fn browse(l: Lang) -> &'static str { get(l, "browse") }
    pub fn browse_title(l: Lang) -> &'static str { get(l, "browse_title") }
    pub fn pack_name_label(l: Lang) -> &'static str { get(l, "pack_name_label") }
    pub fn edition_label(l: Lang) -> &'static str { get(l, "edition_label") }
    pub fn pack_version_label(l: Lang) -> &'static str { get(l, "pack_version_label") }
    pub fn force_new(l: Lang) -> &'static str { get(l, "force_new") }
    pub fn name_preview(l: Lang) -> &'static str { get(l, "name_preview") }
    pub fn scan_button(l: Lang) -> &'static str { get(l, "scan_button") }
    pub fn scanning_in_progress(l: Lang) -> &'static str { get(l, "scanning_in_progress") }
    pub fn dir_not_found(l: Lang) -> &'static str { get(l, "dir_not_found") }
    pub fn language_label(l: Lang) -> &'static str { get(l, "language_label") }
    pub fn no_results(l: Lang) -> &'static str { get(l, "no_results") }
    pub fn run_scan_first(l: Lang) -> &'static str { get(l, "run_scan_first") }
    pub fn results_heading(l: Lang) -> &'static str { get(l, "results_heading") }
    pub fn total_jars(l: Lang) -> &'static str { get(l, "total_jars") }
    pub fn active(l: Lang) -> &'static str { get(l, "active") }
    pub fn disabled(l: Lang) -> &'static str { get(l, "disabled") }
    pub fn read_errors(l: Lang) -> &'static str { get(l, "read_errors") }
    pub fn changes_heading(l: Lang) -> &'static str { get(l, "changes_heading") }
    pub fn no_changes(l: Lang) -> &'static str { get(l, "no_changes") }
    pub fn no_report(l: Lang) -> &'static str { get(l, "no_report") }
    pub fn run_scan_first_short(l: Lang) -> &'static str { get(l, "run_scan_first_short") }
    pub fn generated_markdown(l: Lang) -> &'static str { get(l, "generated_markdown") }
    pub fn copy_to_clipboard(l: Lang) -> &'static str { get(l, "copy_to_clipboard") }
    pub fn copied(l: Lang) -> &'static str { get(l, "copied") }
    pub fn history_heading(l: Lang) -> &'static str { get(l, "history_heading") }
    pub fn refresh(l: Lang) -> &'static str { get(l, "refresh") }
    pub fn no_snapshots(l: Lang) -> &'static str { get(l, "no_snapshots") }
    pub fn run_scan_for_first(l: Lang) -> &'static str { get(l, "run_scan_for_first") }
    pub fn older_snapshot(l: Lang) -> &'static str { get(l, "older_snapshot") }
    pub fn newer_snapshot(l: Lang) -> &'static str { get(l, "newer_snapshot") }
    pub fn select(l: Lang) -> &'static str { get(l, "select") }
    pub fn compare_selected(l: Lang) -> &'static str { get(l, "compare_selected") }
    pub fn select_two_different(l: Lang) -> &'static str { get(l, "select_two_different") }
    pub fn history_comparison(l: Lang) -> &'static str { get(l, "history_comparison") }
    pub fn copy_history_md(l: Lang) -> &'static str { get(l, "copy_history_md") }
    pub fn history_md_copied(l: Lang) -> &'static str { get(l, "history_md_copied") }
    pub fn md_date(l: Lang) -> &'static str { get(l, "md_date") }
    pub fn md_compared_with(l: Lang) -> &'static str { get(l, "md_compared_with") }
    pub fn md_disabled_reason(l: Lang) -> &'static str { get(l, "md_disabled_reason") }
    pub fn md_recommendation(l: Lang) -> &'static str { get(l, "md_recommendation") }

    // ── Formatted strings ───────────────────────────────────────────
    pub fn scan_done(l: Lang, active: usize, disabled: usize, failed: usize, changes: usize) -> String {
        fmt(l, "scan_done", &[
            ("{active}", &active.to_string()),
            ("{disabled}", &disabled.to_string()),
            ("{failed}", &failed.to_string()),
            ("{changes}", &changes.to_string()),
        ])
    }
    pub fn added(l: Lang, count: usize) -> String {
        fmt(l, "added", &[("{count}", &count.to_string())])
    }
    pub fn updated(l: Lang, count: usize) -> String {
        fmt(l, "updated", &[("{count}", &count.to_string())])
    }
    pub fn updated_detail(l: Lang, name: &str, new_ver: &str, old_ver: &str) -> String {
        fmt(l, "updated_detail", &[("{name}", name), ("{new_ver}", new_ver), ("{old_ver}", old_ver)])
    }
    pub fn removed(l: Lang, count: usize) -> String {
        fmt(l, "removed", &[("{count}", &count.to_string())])
    }
    pub fn newly_disabled(l: Lang, count: usize) -> String {
        fmt(l, "newly_disabled", &[("{count}", &count.to_string())])
    }
    pub fn newly_enabled(l: Lang, count: usize) -> String {
        fmt(l, "newly_enabled", &[("{count}", &count.to_string())])
    }
    pub fn unchanged_summary(l: Lang, unchanged: usize, total: usize) -> String {
        fmt(l, "unchanged_summary", &[("{unchanged}", &unchanged.to_string()), ("{total}", &total.to_string())])
    }
    pub fn snapshots_found(l: Lang, count: usize) -> String {
        fmt(l, "snapshots_found", &[("{count}", &count.to_string())])
    }
    pub fn snapshot_read_error(l: Lang, filename: &str) -> String {
        fmt(l, "snapshot_read_error", &[("{filename}", filename)])
    }
    pub fn history_summary(l: Lang, changes: usize, file_a: &str, file_b: &str) -> String {
        fmt(l, "history_summary", &[("{changes}", &changes.to_string()), ("{file_a}", file_a), ("{file_b}", file_b)])
    }

    // ── Markdown generator strings ──────────────────────────────────
    pub fn md_heading(l: Lang, display_name: &str) -> String {
        fmt(l, "md_heading", &[("{name}", display_name)])
    }
    pub fn md_total_mods(l: Lang, active: usize, disabled: usize, failed: usize) -> String {
        fmt(l, "md_total_mods", &[
            ("{active}", &active.to_string()),
            ("{disabled}", &disabled.to_string()),
            ("{failed}", &failed.to_string()),
        ])
    }
    pub fn md_new_mods(l: Lang, count: usize) -> String {
        fmt(l, "md_new_mods", &[("{count}", &count.to_string())])
    }
    pub fn md_updated_mods(l: Lang, count: usize) -> String {
        fmt(l, "md_updated_mods", &[("{count}", &count.to_string())])
    }
    pub fn md_updated_detail(l: Lang, name: &str, new_ver: &str, old_ver: &str) -> String {
        fmt(l, "md_updated_detail", &[("{name}", name), ("{new_ver}", new_ver), ("{old_ver}", old_ver)])
    }
    pub fn md_removed_mods(l: Lang, count: usize) -> String {
        fmt(l, "md_removed_mods", &[("{count}", &count.to_string())])
    }
    pub fn md_newly_disabled(l: Lang, count: usize) -> String {
        fmt(l, "md_newly_disabled", &[("{count}", &count.to_string())])
    }
    pub fn md_newly_enabled(l: Lang, count: usize) -> String {
        fmt(l, "md_newly_enabled", &[("{count}", &count.to_string())])
    }
    pub fn md_currently_disabled(l: Lang, count: usize) -> String {
        fmt(l, "md_currently_disabled", &[("{count}", &count.to_string())])
    }
    pub fn md_read_errors(l: Lang, count: usize) -> String {
        fmt(l, "md_read_errors", &[("{count}", &count.to_string())])
    }
    pub fn md_read_error_detail(l: Lang, filename: &str) -> String {
        fmt(l, "md_read_error_detail", &[("{filename}", filename)])
    }
    pub fn md_summary(l: Lang, unchanged: usize, total: usize) -> String {
        fmt(l, "md_summary", &[("{unchanged}", &unchanged.to_string()), ("{total}", &total.to_string())])
    }
}
