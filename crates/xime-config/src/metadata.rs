//! 应用元数据：libximecore 是跨平台共享库，宿主应用（Xime / XimeChe / Luotuo 等）
//! 通过 [`set_app_metadata`] 注入自身名称，未注入时保持默认 "Xime" 兼容行为。

use std::sync::Mutex;

/// 应用标识信息。所有字段均为 `&'static str`，便于 `const` 构造。
#[derive(Debug, Clone)]
pub struct AppMetadata {
    /// UI 显示名，如 "Xime" / "曦码·逸输入法"。
    pub display_name: &'static str,
    /// 设置窗口标题，如 "Xime 设置" / "曦码·逸输入法设置"。
    ///
    /// 单独给一个字段而不是用 `display_name + "设置"` 拼：中文产品名后面
    /// 通常不留空格、拉丁名后面要留，规则化反而别扭。
    pub settings_name: &'static str,
    /// 侧栏品牌区副标题，如 "输入法设置"。
    pub subtitle: &'static str,
    /// 关于页的一句话简介，如 "基于 Rime 引擎的五笔输入法"。
    pub tagline: &'static str,
    /// 项目主页（关于页「项目主页」按钮打开的地址）。
    pub homepage: &'static str,
    /// 配置目录名（小写），如 "xime"（Linux `~/.config/<name>`、`/usr/share/<name>`）。
    pub config_dir_name: &'static str,
    /// 配置文件基名，如 "xime"（`<base>.yaml`、`<base>.custom.yaml`）。
    pub config_file_base: &'static str,
    /// librime distribution name。
    pub distribution_name: &'static str,
    /// librime distribution code name。
    pub distribution_code_name: &'static str,
    /// librime app name（`rime.<name>.<component>` 惯例）。
    pub app_name: &'static str,
    /// 版本号。
    pub version: &'static str,
}

/// 默认元数据。
///
/// 做成 `static` 而不是每次现造：`app_metadata()` 要返回 `&'static AppMetadata`，
/// 未注入时需要一个**静态**兜底值。
static DEFAULT_METADATA: AppMetadata = AppMetadata {
    display_name: "Xime",
    settings_name: "Xime 设置",
    subtitle: "输入法设置",
    tagline: "基于 Rime 引擎的五笔输入法",
    homepage: "https://github.com/ximeiorg/libximecore",
    config_dir_name: "xime",
    config_file_base: "xime",
    distribution_name: "Xime",
    distribution_code_name: "Xime",
    app_name: "rime.xime.setup",
    version: "1.0",
};

impl Default for AppMetadata {
    fn default() -> Self {
        DEFAULT_METADATA.clone()
    }
}

/// 元数据持有者。
///
/// 为什么不是 `OnceLock`：`OnceLock` 一旦被 `get_or_init` 占住就**不可覆盖**，
/// 而 `app_metadata()` 会在宿主还没来得及注入元数据时就被调用——最典型的就是
/// 日志初始化（`init_logging_with_console` → `get_log_dir` → `get_data_dirs`
/// → `default_rime_paths` → `app_metadata().config_dir_name`）。用 `OnceLock`
/// 时，那次「占住」会让后来的 `set_app_metadata` 直接失败，于是关于页显示的
/// 是默认版本号 "1.0" 而不是宿主真实版本。
///
/// 这里的做法：默认值可以被一次显式注入**覆盖**，只有第二次显式注入才报错。
/// 注入的值 `Box::leak` 成 `&'static`（元数据整个进程生命周期只读）。
struct MetadataCell {
    value: Mutex<&'static AppMetadata>,
    injected: Mutex<bool>,
}

impl MetadataCell {
    const fn new() -> Self {
        Self {
            value: Mutex::new(&DEFAULT_METADATA),
            injected: Mutex::new(false),
        }
    }

    fn get(&self) -> &'static AppMetadata {
        *self.value.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, meta: AppMetadata) -> Result<(), String> {
        {
            let mut injected = self.injected.lock().unwrap_or_else(|e| e.into_inner());
            if *injected {
                return Err("app metadata already set".to_string());
            }
            *injected = true;
        }
        let leaked: &'static AppMetadata = Box::leak(Box::new(meta));
        *self.value.lock().unwrap_or_else(|e| e.into_inner()) = leaked;
        Ok(())
    }
}

static APP_METADATA: MetadataCell = MetadataCell::new();

/// 注入应用元数据。
///
/// 允许在 [`app_metadata`] 已被调用（读到默认值）之后再注入一次——此时才算
/// 真正的宿主元数据生效。第二次及以后的重复注入返回错误。
pub fn set_app_metadata(meta: AppMetadata) -> Result<(), String> {
    APP_METADATA.set(meta)
}

/// 获取应用元数据；未注入时返回默认值（Xime）。
pub fn app_metadata() -> &'static AppMetadata {
    APP_METADATA.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_metadata_is_xime() {
        let meta = AppMetadata::default();
        assert_eq!(meta.display_name, "Xime");
        assert_eq!(meta.settings_name, "Xime 设置");
        assert_eq!(meta.subtitle, "输入法设置");
        assert_eq!(meta.tagline, "基于 Rime 引擎的五笔输入法");
        assert_eq!(meta.homepage, "https://github.com/ximeiorg/libximecore");
        assert_eq!(meta.config_dir_name, "xime");
        assert_eq!(meta.config_file_base, "xime");
        assert_eq!(meta.distribution_name, "Xime");
        assert_eq!(meta.distribution_code_name, "Xime");
        assert_eq!(meta.app_name, "rime.xime.setup");
        assert_eq!(meta.version, "1.0");
    }

    #[test]
    fn test_metadata_is_singleton() {
        let a = app_metadata() as *const _;
        let b = app_metadata() as *const _;
        assert_eq!(a, b);
    }

    /// 回归：读取过默认值之后注入，注入必须生效。
    ///
    /// 宿主（设置页）以前是「先初始化日志、后注入元数据」，日志初始化会经
    /// `get_log_dir` → `get_data_dirs` → `default_rime_paths` 读到
    /// `app_metadata()`，把 OnceLock 占成默认值，之后注入失败 → 关于页显示
    /// "v1.0"。这里用独立的 cell 验证「读默认 → 注入 → 读到新值」成立。
    #[test]
    fn injected_metadata_overrides_default_after_first_read() {
        let cell = MetadataCell::new();
        assert_eq!(cell.get().version, "1.0");

        let meta = AppMetadata {
            display_name: "宿主",
            version: "0.5.0",
            ..AppMetadata::default()
        };
        cell.set(meta).expect("首次注入应成功");
        assert_eq!(cell.get().display_name, "宿主");
        assert_eq!(cell.get().version, "0.5.0");
    }

    /// 回归：第二次注入仍然报错（避免互相覆盖）。
    #[test]
    fn second_injection_is_rejected() {
        let cell = MetadataCell::new();
        cell.set(AppMetadata::default()).expect("首次注入应成功");
        let err = cell.set(AppMetadata::default()).unwrap_err();
        assert!(err.contains("already set"), "{err}");
        // 失败的第二次注入不能改动已生效的值。
        assert_eq!(cell.get().version, "1.0");
    }
}
