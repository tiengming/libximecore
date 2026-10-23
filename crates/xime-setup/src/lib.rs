pub mod app;
pub mod backup;
pub mod components;
pub mod pages;
pub mod state;
pub mod theme;
pub mod webdav;

#[cfg(feature = "voice-page")]
pub mod speech;

/// 本地语音模型（server 侧引擎）在设置端的镜像与回调槽。
#[cfg(feature = "voice-page")]
pub mod speech_models;

pub use app::{run, SettingsApp};
#[cfg(feature = "voice-page")]
pub use speech_models::{
    set_notify_speech_delete, set_notify_speech_download, set_notify_speech_select,
    set_notify_speech_status, set_notify_speech_test_start, set_notify_speech_test_stop,
    SpeechModelEntry, SpeechModelState, SpeechServerStatus,
};
pub use state::SettingsState;
pub use state::{
    set_notify_deploy, set_notify_deploy_toast, set_notify_message, set_notify_reload_plugins,
    set_notify_current_schema, set_notify_reload_style, set_notify_select_schema,
    set_notify_sync_user_data,
};
#[cfg(any(windows, feature = "dict-page"))]
pub use state::{
    set_notify_dict_backup, set_notify_dict_entries, set_notify_dict_entry_write,
    set_notify_dict_export, set_notify_dict_import, set_notify_dict_list, set_notify_dict_restore,
    set_notify_phrase_list, set_notify_phrase_save, set_notify_schema_entries,
};
pub use theme::{SystemTheme, ThemeColors};
pub use xime_config::{
    default_rime_paths, ensure_bundled_rime_data, set_app_metadata, set_rime_paths, AppMetadata,
    RimePaths,
};

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/assets"]
#[include = "icons/*.svg"]
#[include = "image/*.png"]
pub struct Assets;
