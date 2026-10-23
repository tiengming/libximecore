pub mod clipboard_store;
pub mod metadata;
pub mod rime_deploy;
pub mod schema_config;
pub mod schema_manager;
pub mod schema_manifest;
pub mod style;
pub mod user_config;
pub mod wubi_radicals;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing_subscriber::prelude::*;

pub use metadata::{app_metadata, set_app_metadata, AppMetadata};
pub use rime_deploy::{
    default_rime_paths, deploy_all, deploy_all_schemas, ensure_bundled_rime_data, get_data_dirs,
    init_rime_deployer, set_rime_paths, RimePaths, SchemaInfo,
};
pub use schema_config::{
    ReverseLookupConfig, SchemaConfig, SchemaConfigManager, SpellerConfig, TraditionConfig,
    TranslatorConfig,
};
pub use schema_manager::{set_page_size_in, SchemaManager};
pub use schema_manifest::{
    is_protected_release_path, is_trackable_path, is_user_data_path, package_label, FileConflict,
    PackageEntry, Registry, SchemaManifest, UninstallOutcome, BUILTIN_PACKAGE_ID,
};
pub use style::ColorScheme;
pub use style::ColorSchemeConfig;
pub use style::DarkMode;
pub use style::StyleConfig;
pub use user_config::{
    backup_path, original_path, restore_defaults_at, restore_original_at, state_at, undo_at,
    write_overrides_at, ConfigFileState,
};
pub use wubi_radicals::{KeyRadicalsConfig, WubiRadicalsConfig};

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct XimeConfig {
    #[serde(default)]
    pub wubi_radicals: WubiRadicalsConfig,
    #[serde(default)]
    pub style: StyleConfig,
    #[serde(default)]
    pub color_schemes: HashMap<String, ColorScheme>,
    #[serde(default)]
    pub pair_secret: String,
    /// 日志级别（error/warn/info/debug/trace），作用于本项目自己的 crate；
    /// 三方库固定 info（三方 debug 量极大且格式化发生在日志调用线程上）。
    /// 未设或非法值回退 info；进程环境变量 RUST_LOG 仍优先于本配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

impl XimeConfig {
    pub fn load() -> Self {
        Self::load_from(&Self::user_config_path())
    }

    /// 从指定用户文件加载（测试/多实例用；生产走 [`XimeConfig::load`]）。
    ///
    /// 三层按 **YAML 树**深合并：内置 → 系统级 → 用户级。不是按结构体字段替换——
    /// 用户只写 `style.font_size` 时，`style` 的其它字段必须仍是默认值，而不是被
    /// `[serde(default)]` 打回硬编码默认。
    pub fn load_from(user_path: &std::path::Path) -> Self {
        let mut tree = Self::default_tree();
        for path in Self::system_config_paths() {
            if let Some(value) = read_tree(&path) {
                user_config::deep_merge(&mut tree, value);
            }
        }
        if let Some(value) = read_tree(user_path) {
            user_config::deep_merge(&mut tree, value);
        }
        match serde_yaml::from_value(tree) {
            Ok(config) => config,
            Err(_) => Self::builtin_default(),
        }
    }

    /// 内置 + 系统级（**不含**用户文件）——保存差异时的基线。
    pub fn load_defaults() -> Self {
        let mut tree = Self::default_tree();
        for path in Self::system_config_paths() {
            if let Some(value) = read_tree(&path) {
                user_config::deep_merge(&mut tree, value);
            }
        }
        match serde_yaml::from_value(tree) {
            Ok(config) => config,
            Err(_) => Self::builtin_default(),
        }
    }

    fn default_tree() -> serde_yaml::Value {
        const DEFAULT_CONFIG: &[u8] = include_bytes!("../../../resources/xime.yaml");
        serde_yaml::from_slice(DEFAULT_CONFIG).unwrap_or(serde_yaml::Value::Null)
    }

    fn builtin_default() -> Self {
        const DEFAULT_CONFIG: &[u8] = include_bytes!("../../../resources/xime.yaml");
        serde_yaml::from_slice(DEFAULT_CONFIG).unwrap_or_default()
    }
    fn system_config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let meta = app_metadata();
        let config_dir = meta.config_dir_name;
        let config_file = format!("{}.yaml", meta.config_file_base);

        // Linux: /usr/share/xime/xime.yaml
        if cfg!(unix) {
            paths.push(
                PathBuf::from("/usr/share")
                    .join(config_dir)
                    .join(&config_file),
            );
        }

        // Windows: data/xime.yaml next to exe
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                paths.push(parent.join("data").join(&config_file));
                paths.push(parent.join("resources").join(&config_file));
            }
        }

        paths
    }

    pub fn user_config_path() -> PathBuf {
        let meta = app_metadata();
        let config_dir = meta.config_dir_name;
        let custom_file = format!("{}.custom.yaml", meta.config_file_base);
        let config_file = format!("{}.yaml", meta.config_file_base);

        // macOS: ~/Library/Application Support/Xime/xime.custom.yaml
        if cfg!(target_os = "macos") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            let base = PathBuf::from(&home)
                .join("Library/Application Support")
                .join(config_dir);
            for path in &[base.join(&custom_file), base.join(&config_file)] {
                if path.exists() {
                    return path.clone();
                }
            }
            return base.join(custom_file);
        }

        // Linux: ~/.config/xime/xime.custom.yaml or xime.yaml
        if cfg!(unix) {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            let base = PathBuf::from(&home).join(".config").join(config_dir);
            for path in &[
                base.join(&custom_file),
                base.join("rime").join(&custom_file),
                base.join(&config_file),
                base.join("rime").join(&config_file),
            ] {
                if path.exists() {
                    return path.clone();
                }
            }
            return base.join(custom_file);
        }

        // Windows: %APPDATA%/Xime/rime/xime.yaml
        if let Ok(appdata) = std::env::var("APPDATA") {
            let base = PathBuf::from(&appdata).join(config_dir);
            for path in &[
                base.join("rime").join(&custom_file),
                base.join("rime").join(&config_file),
            ] {
                if path.exists() {
                    return path.clone();
                }
            }
            return base.join("rime").join(custom_file);
        }

        PathBuf::from(custom_file)
    }

    pub fn config_path() -> PathBuf {
        Self::user_config_path()
    }

    /// 保存用户配置：**只写与默认值的差异**，并保留用户手写的其它键。
    ///
    /// 之前的实现是 `serde_yaml::to_string(self)` 整份覆盖：外观页点一下就会把
    /// 5KB 的 `wubi_radicals`、全套内置配色、甚至 `pair_secret` 全灌进用户文件，
    /// 用户自己写的段（移动端的 `keyboard:` 等）会被直接抹掉。
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::config_path())
    }

    /// 见 [`XimeConfig::save`]；路径可指定（测试用）。
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        let mine = serde_yaml::to_value(self)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        let baseline = serde_yaml::to_value(Self::load_defaults())
            .map_err(|e| format!("Failed to serialize defaults: {e}"))?;

        // 我们负责的顶层键 = 结构体能序列化出来的键集合（新增字段自动纳管）。
        let managed: Vec<String> = match &mine {
            serde_yaml::Value::Mapping(map) => map
                .keys()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };

        let overrides = user_config::deep_diff(&mine, &baseline);
        user_config::write_overrides_at(path, overrides, &managed)
    }

    /// 用户配置文件状态（是否存在 / 可否撤销 / 是否有用户初始版本）。
    pub fn file_state() -> user_config::ConfigFileState {
        user_config::state_at(&Self::config_path())
    }

    /// 撤销上一次保存（用 `.bak` 覆盖回去）。
    pub fn undo_last_save() -> Result<(), String> {
        user_config::undo_at(&Self::config_path())
    }

    /// 恢复为客户端默认：删除用户文件（= 回到内置 + 系统级）。
    pub fn restore_defaults() -> Result<(), String> {
        user_config::restore_defaults_at(&Self::config_path())
    }

    /// 恢复为用户最初提供的版本（第一次被程序改写前的那份 `.orig`）。
    pub fn restore_original() -> Result<(), String> {
        user_config::restore_original_at(&Self::config_path())
    }

    pub fn get_primary_color(&self) -> (u8, u8, u8) {
        // Default to light mode
        self.get_primary_color_for_theme(false)
    }

    pub fn get_primary_color_for_theme(&self, system_is_dark: bool) -> (u8, u8, u8) {
        let is_dark = self.style.dark_mode.is_dark(system_is_dark);
        let scheme_name = self.style.color_scheme.scheme_name(is_dark);
        if let Some(scheme) = self.color_schemes.get(&scheme_name) {
            let r = (scheme.primary_color >> 16) as u8;
            let g = (scheme.primary_color >> 8) as u8;
            let b = scheme.primary_color as u8;
            (r, g, b)
        } else {
            (0x8F, 0x73, 0xE2)
        }
    }

    /// Get the resolved color scheme name based on dark_mode setting.
    pub fn get_color_scheme_name(&self, system_is_dark: bool) -> String {
        let is_dark = self.style.dark_mode.is_dark(system_is_dark);
        self.style.color_scheme.scheme_name(is_dark)
    }

    /// Get keyboard background color for the current theme.
    pub fn get_keyboard_bg_color(&self, system_is_dark: bool) -> Option<u32> {
        let is_dark = self.style.dark_mode.is_dark(system_is_dark);
        let scheme_name = self.style.color_scheme.scheme_name(is_dark);
        if let Some(scheme) = self.color_schemes.get(&scheme_name) {
            if let Some(ref bg) = scheme.keyboard_background {
                if is_dark {
                    return bg.color_dark;
                } else {
                    return bg.color;
                }
            }
        }
        None
    }

    /// Get key background color for the current theme.
    pub fn get_key_bg_color(&self, system_is_dark: bool) -> Option<u32> {
        let is_dark = self.style.dark_mode.is_dark(system_is_dark);
        let scheme_name = self.style.color_scheme.scheme_name(is_dark);
        if let Some(scheme) = self.color_schemes.get(&scheme_name) {
            if is_dark {
                return scheme.key_bg_color_dark;
            } else {
                return scheme.key_bg_color;
            }
        }
        None
    }

    /// Get wubi root text for a key press in the given schema.
    /// Delegates to `wubi_radicals`.
    pub fn get_root_for_key(&self, schema: &str, key: char) -> Option<String> {
        self.wubi_radicals.get_root_for_key(schema, key)
    }

    /// Get the key binding that triggers the wubi radicals overlay.
    /// Returns e.g. "Ctrl" or "Shift" from `wubi_radicals.hotkeys.show_key`.
    pub fn get_last_key_root_binding(&self) -> String {
        self.wubi_radicals.hotkeys.show_key.clone()
    }

    /// 规范化后的日志级别：未设/非法回退 "info"，大小写不敏感、去首尾空白。
    pub fn effective_log_level(&self) -> &str {
        let raw = self.log_level.as_deref().unwrap_or("").trim();
        match raw.to_ascii_lowercase().as_str() {
            "error" => "error",
            "warn" => "warn",
            "debug" => "debug",
            "trace" => "trace",
            _ => "info",
        }
    }
}

/// 读一份 YAML 文件成值树；不存在/解析失败都当"没有这一层"。
fn read_tree(path: &std::path::Path) -> Option<serde_yaml::Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&text).ok()
}

// Logging support (cross-platform)
static LOG_GUARD: Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> = Mutex::new(None);

/// 原子覆盖写：先在同目录写临时文件（unix 下 0600，防多用户机器上的
/// 短暂可读窗口）再 rename。崩溃/掉电最坏留下一个 `.tmp.<pid>` 残件，
/// 目标文件要么是旧内容要么是新内容，不会是半截——所有"读-改-写"的
/// 用户配置（短语表、词典补丁、插件注册表、方案清单）都应走这里。
pub fn atomic_write(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_string_lossy();
    let tmp = dir.join(format!(".{file_name}.tmp.{}", std::process::id()));

    {
        let mut f = open_private_file(&tmp)?;
        f.write_all(data)?;
    }
    fs::rename(&tmp, path)
}

/// 以 0600 新建/截断文件（unix）；其他平台按默认权限。
fn open_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }
}

/// 默认日志过滤：
/// - **三方库**（wgpu / winit / glyphon / fontdb / hyper…）只到 `info`。它们的
///   debug 日志量极大（实测设置程序一次会话 12.9 MB / 22.5 万行，含每帧
///   `relayout`、图集分配、字体回退探测），而 tracing 的**格式化发生在写日志
///   的那个线程**——设置程序里就是 UI 线程，等于白送每帧开销，还会把真正
///   有用的启动耗时埋掉；
/// - **本项目自己的 crate** 仍保留 `debug`，排障信息不丢。
///
/// 需要全量日志时用环境变量覆盖：`RUST_LOG=debug`。
const DEFAULT_LOG_FILTER: &str = concat!(
    "info",
    ",xime_config=debug,xime_setup_lib=debug,xime_plugin=debug,xime_rime=debug,xime_ipc=debug",
    ",winxime_server=debug,winxime_setup=debug,winxime_tsf=debug,winxime_ipc=debug"
);

pub fn init_logging(component: &str) {
    let log_dir = get_log_dir();
    fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::never(&log_dir, format!("{}.log", component));

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    if let Ok(mut g) = LOG_GUARD.lock() {
        *g = Some(guard);
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false)
        .with_line_number(true)
        .try_init()
        .ok();
}

pub fn init_logging_with_console(component: &str) {
    let log_dir = get_log_dir();
    fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::never(&log_dir, format!("{}.log", component));

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    if let Ok(mut g) = LOG_GUARD.lock() {
        *g = Some(guard);
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(false)
                .with_line_number(true),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .with_ansi(true),
        )
        .try_init()
        .ok();
}

fn get_log_dir() -> PathBuf {
    if cfg!(windows) {
        // 数据根 logs\（%APPDATA%\<name>\logs），与用户数据同处，不随 TEMP 清理丢失。
        get_data_dirs()
            .1
            .parent()
            .map(|p| p.join("logs"))
            .unwrap_or_else(|| {
                std::env::var("TEMP")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(app_metadata().config_dir_name)
            })
    } else {
        dirs_or_home().join("log")
    }
}

fn dirs_or_home() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local/share")
            .join(app_metadata().config_dir_name)
    } else {
        PathBuf::from("/tmp").join(app_metadata().config_dir_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 日志级别解析：非法值必须静默回退 info 而不是让整个过滤器失效。
    #[test]
    fn effective_log_level_validation() {
        for (raw, want) in [
            (None, "info"),
            (Some("debug"), "debug"),
            (Some(" DEBUG "), "debug"),
            (Some("Trace"), "trace"),
            (Some("warn"), "warn"),
            (Some("error"), "error"),
            (Some(""), "info"),
            (Some("verbose"), "info"),
            (Some("debug; rm -rf"), "info"),
        ] {
            let cfg = XimeConfig {
                log_level: raw.map(str::to_string),
                ..XimeConfig::default()
            };
            assert_eq!(cfg.effective_log_level(), want, "raw = {raw:?}");
        }
    }

    /// 合并语义：上层配置显式设置了 log_level 才覆盖；未设时保留下层值。
    ///
    /// 现在合并发生在 YAML 树层面（`deep_merge`），所以"未设"就是"键不存在"；
    /// 序列化时 `log_level` 带 `skip_serializing_if`，天然不会出现空值覆盖。
    #[test]
    fn merge_log_level_precedence() {
        let merge = |base: serde_yaml::Value, over: serde_yaml::Value| -> XimeConfig {
            let mut tree = base;
            user_config::deep_merge(&mut tree, over);
            serde_yaml::from_value(tree).unwrap()
        };
        let with = |level: &str| {
            serde_yaml::to_value(XimeConfig {
                log_level: Some(level.to_string()),
                ..XimeConfig::default()
            })
            .unwrap()
        };

        // 上层设了 → 覆盖
        let merged = merge(with("info"), with("debug"));
        assert_eq!(merged.effective_log_level(), "debug");

        // 上层没设 → 保留下层的 debug（不能被空值打回 info）
        let merged = merge(
            with("debug"),
            serde_yaml::to_value(XimeConfig::default()).unwrap(),
        );
        assert_eq!(merged.effective_log_level(), "debug");
    }

    /// 序列化不回写未设置的 log_level（setup 保存配置不应引入 null/空键）。
    #[test]
    fn log_level_absent_is_not_serialized() {
        let cfg = XimeConfig::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(!yaml.contains("log_level"), "未设置时不应序列化: {yaml}");

        let cfg = XimeConfig {
            log_level: Some("debug".to_string()),
            ..XimeConfig::default()
        };
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(yaml.contains("log_level: debug"));
    }

    /// 默认日志过滤必须能被 EnvFilter 解析：写错指令时 `EnvFilter::new` 只是
    /// 静默丢掉那一条，会把我们自己的 debug 日志一起吞掉而无人察觉。
    #[test]
    fn default_log_filter_is_valid_and_keeps_own_crates_verbose() {
        let filter = tracing_subscriber::EnvFilter::try_new(DEFAULT_LOG_FILTER);
        assert!(filter.is_ok(), "默认日志过滤无法解析: {DEFAULT_LOG_FILTER}");

        // 只断言「本项目 crate 有 debug 指令」这种能静态检查的部分：
        // 三方库默认 info，本项目 crate 逐个列出 debug。
        assert!(
            DEFAULT_LOG_FILTER.starts_with("info"),
            "三方库默认级别必须是 info（否则每帧日志会拖慢 UI 线程）"
        );
        for target in [
            "xime_config",
            "xime_setup_lib",
            "xime_rime",
            "winxime_server",
            "winxime_setup",
            "winxime_tsf",
        ] {
            assert!(
                DEFAULT_LOG_FILTER.contains(&format!("{target}=debug")),
                "缺少本项目 crate 的 debug 指令: {target}"
            );
        }
    }
}
