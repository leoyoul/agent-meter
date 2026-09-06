use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuMetrics {
    #[serde(default = "enabled_by_default")]
    pub today_tokens: bool,
    #[serde(default)]
    pub ttft: bool,
    #[serde(default)]
    pub effective_tps: bool,
}

impl Default for MenuMetrics {
    fn default() -> Self {
        Self {
            today_tokens: true,
            ttft: false,
            effective_tps: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    #[serde(default = "enabled_by_default")]
    pub automatic_check: bool,
    #[serde(default)]
    pub last_checked_at: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            automatic_check: true,
            last_checked_at: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    #[serde(default)]
    pub menu_metrics: MenuMetrics,
    #[serde(default)]
    pub updates: UpdateSettings,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Clone)]
pub struct SettingsState {
    path: PathBuf,
}

impl SettingsState {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> AppSettings {
        match load_settings(&self.path) {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("Agent Meter: {error}");
                AppSettings::default()
            }
        }
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        save_settings(&self.path, settings)
    }
}

fn load_settings(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let bytes = fs::read(path).map_err(|error| format!("读取设置失败: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("设置文件损坏，已恢复默认值: {error}"))
}

fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建设置目录失败: {error}"))?;
    }
    let temp_path = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|error| format!("序列化设置失败: {error}"))?;
    fs::write(&temp_path, bytes).map_err(|error| format!("写入设置失败: {error}"))?;
    fs::rename(&temp_path, path).map_err(|error| format!("保存设置失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_keep_only_today_tokens_visible() {
        let settings = AppSettings::default();
        assert!(settings.menu_metrics.today_tokens);
        assert!(!settings.menu_metrics.ttft);
        assert!(!settings.menu_metrics.effective_tps);
        assert!(settings.updates.automatic_check);
    }

    #[test]
    fn persists_and_reloads_settings() {
        let temp = tempdir().unwrap();
        let state = SettingsState::new(temp.path().join("settings.json"));
        let mut settings = AppSettings::default();
        settings.menu_metrics.ttft = true;
        settings.updates.last_checked_at = Some("2026-09-06T12:00:00Z".into());
        state.save(&settings).unwrap();
        assert_eq!(state.load(), settings);
    }

    #[test]
    fn malformed_settings_fall_back_to_defaults() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, b"not json").unwrap();
        let state = SettingsState::new(path);
        assert_eq!(state.load(), AppSettings::default());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, br#"{"menuMetrics":{"ttft":true}}"#).unwrap();
        let settings = SettingsState::new(path).load();
        assert!(settings.menu_metrics.today_tokens);
        assert!(settings.menu_metrics.ttft);
        assert!(settings.updates.automatic_check);
    }
}
