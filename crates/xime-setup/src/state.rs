use crate::theme::{SystemTheme, ThemeColors};
use serde::Deserialize;
use sha2::Digest;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use xime_config::schema_manifest::{
    self, Registry, SchemaManifest, BUILTIN_PACKAGE_ID, LOCAL_PACKAGE_ID,
};
use xime_config::{
    deploy_all, get_data_dirs, ColorSchemeConfig, DarkMode, SchemaConfig, SchemaConfigManager,
    SchemaInfo, SchemaManager, XimeConfig,
};

static MARKET_TASK_RESULT: OnceLock<Mutex<Option<MarketTaskResult>>> = OnceLock::new();
static MARKET_YAML_RESULT: OnceLock<Mutex<Option<Result<String, String>>>> = OnceLock::new();
static DEPLOY_RESULT: OnceLock<Mutex<Option<Result<String, String>>>> = OnceLock::new();
static MODEL_TASK_RESULT: OnceLock<Mutex<Option<ModelTaskResult>>> = OnceLock::new();
static MODEL_YAML_RESULT: OnceLock<Mutex<Option<Result<String, String>>>> = OnceLock::new();
static PLUGIN_TASK_RESULT: OnceLock<Mutex<Option<PluginTaskResult>>> = OnceLock::new();
static PLUGIN_YAML_RESULT: OnceLock<Mutex<Option<Result<String, String>>>> = OnceLock::new();
static DOWNLOAD_PROGRESS: OnceLock<Mutex<Option<(String, f32)>>> = OnceLock::new();

/// 本地方案导入的后台准备结果（复制/下载完成后的包 id）。
static SCHEMA_IMPORT_RESULT: OnceLock<Mutex<Option<Result<String, String>>>> = OnceLock::new();
static SCHEMA_IMPORT_PROGRESS: OnceLock<Mutex<Option<(u64, u64, u64)>>> = OnceLock::new();

fn schema_import_result() -> &'static Mutex<Option<Result<String, String>>> {
    SCHEMA_IMPORT_RESULT.get_or_init(|| Mutex::new(None))
}

fn schema_import_progress() -> &'static Mutex<Option<(u64, u64, u64)>> {
    SCHEMA_IMPORT_PROGRESS.get_or_init(|| Mutex::new(None))
}

/// 方案列表快照：安装/卸载后由后台线程重扫，UI 只做一次替换，不在消息循环里
/// 逐 `*.schema.yaml` 读文件 + 解析（审计 P0「轮询里的 reload_schemas」）。
struct SchemaSnapshot {
    schemas: Vec<SchemaInfo>,
    packages: Vec<String>,
    builtin_restorable: bool,
    deployed: Vec<String>,
}

static SCHEMA_SNAPSHOT: OnceLock<Mutex<Option<SchemaSnapshot>>> = OnceLock::new();
static SCHEMA_RELOADING: AtomicBool = AtomicBool::new(false);

fn schema_snapshot() -> &'static Mutex<Option<SchemaSnapshot>> {
    SCHEMA_SNAPSHOT.get_or_init(|| Mutex::new(None))
}

fn market_task_result() -> &'static Mutex<Option<MarketTaskResult>> {
    MARKET_TASK_RESULT.get_or_init(|| Mutex::new(None))
}

fn market_yaml_result() -> &'static Mutex<Option<Result<String, String>>> {
    MARKET_YAML_RESULT.get_or_init(|| Mutex::new(None))
}

fn deploy_result() -> &'static Mutex<Option<Result<String, String>>> {
    DEPLOY_RESULT.get_or_init(|| Mutex::new(None))
}

fn model_task_result() -> &'static Mutex<Option<ModelTaskResult>> {
    MODEL_TASK_RESULT.get_or_init(|| Mutex::new(None))
}

fn model_yaml_result() -> &'static Mutex<Option<Result<String, String>>> {
    MODEL_YAML_RESULT.get_or_init(|| Mutex::new(None))
}

fn plugin_task_result() -> &'static Mutex<Option<PluginTaskResult>> {
    PLUGIN_TASK_RESULT.get_or_init(|| Mutex::new(None))
}

fn plugin_yaml_result() -> &'static Mutex<Option<Result<String, String>>> {
    PLUGIN_YAML_RESULT.get_or_init(|| Mutex::new(None))
}

/// 下载进度（任务 id, 0.0~1.0），由下载线程更新、UI 轮询取走。
fn download_progress() -> &'static Mutex<Option<(String, f32)>> {
    DOWNLOAD_PROGRESS.get_or_init(|| Mutex::new(None))
}

enum MarketTaskResult {
    DownloadDone(String),
    /// 安装完成（已安装包列表按注册表重建，无需回传 id）。
    InstallDone,
    /// 卸载完成（同上）。
    UninstallDone,
    DeleteDone(String),
    /// 内置方案包从 market/builtin/ 备份还原（文件数）。
    BuiltinRestored(usize),
    /// 内置方案包清理完成（文件数）。
    BuiltinCleaned(usize),
    Error(String),
}

enum ModelTaskResult {
    DownloadDone(String),
    DeleteDone(String),
    Error(String),
}

enum PluginTaskResult {
    InstallDone(String),
    UninstallDone(String),
    ToggleDone(String, bool),
    Error(String),
}

static NOTIFY_DEPLOY: OnceLock<fn()> = OnceLock::new();
static NOTIFY_RELOAD_STYLE: OnceLock<fn()> = OnceLock::new();
static NOTIFY_SELECT_SCHEMA: OnceLock<fn(&str) -> bool> = OnceLock::new();
static NOTIFY_MESSAGE: OnceLock<fn(&str, &str)> = OnceLock::new();
static NOTIFY_RELOAD_PLUGINS: OnceLock<fn()> = OnceLock::new();
static NOTIFY_SYNC_USER_DATA: OnceLock<fn() -> bool> = OnceLock::new();
/// 读宿主**实际**在用的方案 id（服务端 `status.schema_id`）。
/// 未注册回调或服务未运行 → None。「当前方案」不能只靠本地下标猜。
static NOTIFY_CURRENT_SCHEMA: OnceLock<fn() -> Option<String>> = OnceLock::new();
/// 部署结果系统通知（平台通知由宿主实现；libximecore 保持平台无关）。
static NOTIFY_DEPLOY_TOAST: OnceLock<fn(&str, &str)> = OnceLock::new();

/// 设置宿主进程的「部署结果系统通知」回调（宿主实现 Windows toast 等平台通知）。
pub fn set_notify_deploy_toast(f: fn(&str, &str)) {
    let _ = NOTIFY_DEPLOY_TOAST.set(f);
}

fn notify_deploy_toast(title: &str, body: &str) {
    if let Some(f) = NOTIFY_DEPLOY_TOAST.get() {
        f(title, body);
    }
}

/// 本地方案导入：浏览文件对话框结果（后台线程，避免大目录卡住 UI）。
static SCHEMA_IMPORT_BROWSE: OnceLock<Mutex<Option<Option<std::path::PathBuf>>>> = OnceLock::new();

fn schema_import_browse() -> &'static Mutex<Option<Option<std::path::PathBuf>>> {
    SCHEMA_IMPORT_BROWSE.get_or_init(|| Mutex::new(None))
}

/// 后台：部署 → 通知宿主重载（重建会话）→ 再显式选中目标方案。
///
/// 只在「目标方案还没有 build 产物」时走这条路：宿主的 `SelectSchema` 会拒绝
/// 未部署的方案（选进未部署方案会得到死会话），所以必须先把产物部署出来，
/// 再让宿主重新部署并补一次显式选中，切换才真正落到正在打字的引擎上。
///
/// 整段同步做要数秒（全量维护 + redeploy），绝不能进 UI 线程；结果写进
/// `deploy_result`，由 `poll_deploy` 轮询后提示。
///
/// 返回是否真的排上了任务（已有部署在跑时返回 false，调用方据此换提示语）。
fn start_deploy_then_select(schema_id: &str, schema_name: &str) -> bool {
    // 占住部署槽位：既避免与「部署方案」按钮并发部署，也顺手给出即时反馈。
    match deploy_result().lock() {
        Ok(mut slot) if slot.is_none() => {
            *slot = Some(Ok("当前方案未部署，正在部署，请等待…".to_string()))
        }
        _ => return false,
    }
    notify_deploy_toast(
        "方案切换",
        &format!("「{schema_name}」尚未部署，正在部署，请等待…"),
    );

    let id = schema_id.to_string();
    let name = schema_name.to_string();
    std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            deploy_all().map_err(|e| format!("部署失败: {}", e))?;
            if !notify_daemon_reload() {
                return Ok(format!("已部署「{name}」；服务器未运行，下次启动时生效"));
            }
            // server 收到 ReloadConfig 后是异步重建会话的，立刻 SelectSchema 仍可能
            // 被拒；给引擎几拍时间，避免把「还没准备好」误报成切换失败。
            for attempt in 0..6 {
                if notify_select_schema(&id) {
                    return Ok(format!("已部署并切换到「{name}」"));
                }
                if attempt < 5 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
            Err(format!("部署完成，但服务器仍未切换到「{name}」"))
        })();
        match &result {
            Ok(msg) => notify_deploy_toast("方案切换完成", msg),
            Err(e) => notify_deploy_toast("方案切换失败", e),
        }
        if let Ok(mut slot) = deploy_result().lock() {
            *slot = Some(result);
        }
    });
    true
}

/// 设置宿主进程的「部署后重载」回调（daemon 重载配置）。
pub fn set_notify_deploy(f: fn()) {
    let _ = NOTIFY_DEPLOY.set(f);
}

/// 设置宿主进程的「样式重载」回调（daemon 重新加载配色/字号）。
pub fn set_notify_reload_style(f: fn()) {
    let _ = NOTIFY_RELOAD_STYLE.set(f);
}

/// 设置宿主进程的「切换当前输入方案」回调（通过 IPC 发送 SelectSchema 命令）。
/// 返回是否发送成功（服务器是否运行）。
pub fn set_notify_select_schema(f: fn(&str) -> bool) {
    let _ = NOTIFY_SELECT_SCHEMA.set(f);
}

/// 设置宿主进程的「结果消息」回调（部署/保存等成功失败提示）。
/// 宿主可用它发系统通知；未注册则消息仅在页面底部显示。
pub fn set_notify_message(f: fn(&str, &str)) {
    let _ = NOTIFY_MESSAGE.set(f);
}

/// 设置宿主进程的「插件变更」回调（daemon 重载插件：安装/卸载/启停后触发）。
pub fn set_notify_reload_plugins(f: fn()) {
    let _ = NOTIFY_RELOAD_PLUGINS.set(f);
}

/// 设置宿主进程的「用户资料同步」回调（IPC SyncUserData，阻塞等待完成）。
pub fn set_notify_sync_user_data(f: fn() -> bool) {
    let _ = NOTIFY_SYNC_USER_DATA.set(f);
}

/// 注册宿主进程的「读当前方案 id」回调（服务端 status.schema_id）。
/// 设置页用它把「当前方案」标在真正在用的那一行上——列表是按显示名排序的，
/// 光靠下标会一直标到字母序最前的方案（比如 Emoji）。
pub fn set_notify_current_schema(f: fn() -> Option<String>) {
    let _ = NOTIFY_CURRENT_SCHEMA.set(f);
}

/// 触发 rime 用户资料同步；false = 服务器未运行或同步失败。
#[cfg(feature = "backup-page")]
pub fn notify_sync_user_data() -> bool {
    NOTIFY_SYNC_USER_DATA.get().map(|f| f()).unwrap_or(false)
}

/// 读宿主当前方案 id；未注册/服务未运行返回 None。
fn notify_current_schema() -> Option<String> {
    NOTIFY_CURRENT_SCHEMA.get().and_then(|f| f())
}

// ---- 词典管理回调（host 注册，Windows；对齐 weasel DictManagementDialog）----

/// 用户词典列表结果（词典名 + 快照目录）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct DictListResult {
    pub dicts: Vec<String>,
    pub sync_dir: String,
}

/// 用户词典中的一条词条（词 / 编码 / 频率）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct DictEntryRow {
    pub word: String,
    pub code: String,
    pub commits: i32,
}

/// 词条读取结果（词库总数 + 命中数 + 本次返回的词条）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct DictEntriesResult {
    /// 词库词条总数（未受关键词过滤影响）。
    pub total: i32,
    /// 命中条数（**未**受回传上限影响，用来判断回传是否被截断）。
    pub matched: i32,
    /// 本次返回的词条（已按关键词过滤，最多 `DICT_ENTRIES_MAX` 条）。
    pub entries: Vec<DictEntryRow>,
}

/// 单次读取最多返回的词条数。
///
/// 必须与 IPC 侧的 `winxime_ipc::MAX_DICT_ENTRIES` 保持一致：那边受命名管道
/// 单帧上限约束会截断，这里用来提示"命中过多，请补充关键词"。
#[cfg(any(windows, feature = "dict-page"))]
pub const DICT_ENTRIES_MAX: usize = 500;

/// 写入一条用户词条的结果（新增 / 删除标记都算写）。
///
/// 带词典名：在途期间切换词典下拉时，结果按词典名丢弃（但 `writing`
/// 标志仍要清掉——见 poll 的处理）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct DictWriteResult {
    /// 写入的目标词典。
    pub dict: String,
    /// 是否成功。
    pub ok: bool,
    /// 给用户看的结果文案。
    pub message: String,
}

/// 方案词表读取结果（只读浏览）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct SchemaEntriesResult {
    /// 方案主码表名（`dictionary:` 的值）。
    pub dict_name: String,
    /// 实际读入的码表名（主表 + import_tables + packs）。
    pub tables: Vec<String>,
    /// 声明了但文件不存在的码表名。
    pub missing: Vec<String>,
    /// 读入的词条总数。
    pub total: i32,
    /// 命中条数（未受回传上限影响）。
    pub matched: i32,
    /// 本次返回的词条（方案词表没有频率概念，`commits` 恒为 0）。
    pub entries: Vec<DictEntryRow>,
}

/// 快捷短语的一条（词 / 编码 / 可选权重）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct CustomPhraseRow {
    /// 短语文本。
    pub word: String,
    /// 触发编码。
    pub code: String,
    /// 权重（正整数；`None` = 文件里省略这一列，走 rime 默认权重）。
    pub weight: Option<i32>,
}

/// 快捷短语表读取结果。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct PhraseListResult {
    /// 短语表名（`custom_phrase.user_dict`，通常就是 `custom_phrase`）。
    pub dict_name: String,
    /// 短语表文件名（`<表名>.txt`）。
    pub file_name: String,
    /// 短语表文件是否已存在。
    pub file_exists: bool,
    /// 方案 custom.yaml 里是否已注入翻译器。
    pub patch_applied: bool,
    /// 短语列表。
    pub entries: Vec<CustomPhraseRow>,
}

/// 快捷短语整表保存结果。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct PhraseSaveResult {
    pub dict_name: String,
    pub file_name: String,
    pub file_exists: bool,
    pub patch_applied: bool,
    /// 本次保存是否**新**注入了翻译器（需要重新部署才生效）。
    pub patch_added: bool,
    /// 保存后的整表。
    pub entries: Vec<CustomPhraseRow>,
}

#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_LIST: OnceLock<fn() -> Option<DictListResult>> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_BACKUP: OnceLock<fn(&str) -> bool> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_RESTORE: OnceLock<fn(&str) -> bool> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_EXPORT: OnceLock<fn(&str, &str) -> Option<i32>> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_IMPORT: OnceLock<fn(&str, &str) -> Option<i32>> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_DICT_ENTRIES: OnceLock<fn(&str, &str) -> Option<DictEntriesResult>> = OnceLock::new();

/// 设置宿主进程的「列出用户词典」回调（IPC ListUserDicts）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_list(f: fn() -> Option<DictListResult>) {
    let _ = NOTIFY_DICT_LIST.set(f);
}

/// 设置宿主进程的「备份用户词典」回调（IPC BackupUserDict）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_backup(f: fn(&str) -> bool) {
    let _ = NOTIFY_DICT_BACKUP.set(f);
}

/// 设置宿主进程的「恢复用户词典」回调（IPC RestoreUserDict）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_restore(f: fn(&str) -> bool) {
    let _ = NOTIFY_DICT_RESTORE.set(f);
}

/// 设置宿主进程的「导出用户词典」回调（IPC ExportUserDict，返回条数）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_export(f: fn(&str, &str) -> Option<i32>) {
    let _ = NOTIFY_DICT_EXPORT.set(f);
}

/// 设置宿主进程的「导入用户词典」回调（IPC ImportUserDict，返回条数）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_import(f: fn(&str, &str) -> Option<i32>) {
    let _ = NOTIFY_DICT_IMPORT.set(f);
}

/// 设置宿主进程的「读取用户词典词条」回调（IPC ListDictEntries，参数为词典名 + 关键词）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_entries(f: fn(&str, &str) -> Option<DictEntriesResult>) {
    let _ = NOTIFY_DICT_ENTRIES.set(f);
}

#[cfg(any(windows, feature = "dict-page"))]
#[allow(clippy::type_complexity)]
static NOTIFY_DICT_ENTRY_WRITE: OnceLock<fn(&str, &str, &str, i32) -> Option<i32>> =
    OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
#[allow(clippy::type_complexity)]
static NOTIFY_SCHEMA_ENTRIES: OnceLock<fn(&str, &str) -> Option<SchemaEntriesResult>> =
    OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
static NOTIFY_PHRASE_LIST: OnceLock<fn(&str) -> Option<PhraseListResult>> = OnceLock::new();
#[cfg(any(windows, feature = "dict-page"))]
#[allow(clippy::type_complexity)]
static NOTIFY_PHRASE_SAVE: OnceLock<fn(&str, &[CustomPhraseRow]) -> Option<PhraseSaveResult>> =
    OnceLock::new();

/// 设置宿主进程的「写入一条用户词条」回调（IPC ImportDictEntry，
/// 参数为词典名 / 词 / 编码 / 频率，频率 < 0 即删除标记，返回导入条数）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_dict_entry_write(f: fn(&str, &str, &str, i32) -> Option<i32>) {
    let _ = NOTIFY_DICT_ENTRY_WRITE.set(f);
}

/// 设置宿主进程的「读取方案词表词条」回调（IPC ListSchemaEntries，参数为方案 id + 关键词）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_schema_entries(f: fn(&str, &str) -> Option<SchemaEntriesResult>) {
    let _ = NOTIFY_SCHEMA_ENTRIES.set(f);
}

/// 设置宿主进程的「读取快捷短语表」回调（IPC ListCustomPhrases，参数为方案 id）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_phrase_list(f: fn(&str) -> Option<PhraseListResult>) {
    let _ = NOTIFY_PHRASE_LIST.set(f);
}

/// 设置宿主进程的「整表保存快捷短语」回调（IPC SaveCustomPhrases，
/// 参数为方案 id + 整张短语表，覆盖式写入）。
#[cfg(any(windows, feature = "dict-page"))]
pub fn set_notify_phrase_save(f: fn(&str, &[CustomPhraseRow]) -> Option<PhraseSaveResult>) {
    let _ = NOTIFY_PHRASE_SAVE.set(f);
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_list() -> Option<DictListResult> {
    NOTIFY_DICT_LIST.get().and_then(|f| f())
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_backup(dict: &str) -> bool {
    NOTIFY_DICT_BACKUP.get().map(|f| f(dict)).unwrap_or(false)
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_restore(path: &str) -> bool {
    NOTIFY_DICT_RESTORE.get().map(|f| f(path)).unwrap_or(false)
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_export(dict: &str, path: &str) -> Option<i32> {
    NOTIFY_DICT_EXPORT.get().and_then(|f| f(dict, path))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_import(dict: &str, path: &str) -> Option<i32> {
    NOTIFY_DICT_IMPORT.get().and_then(|f| f(dict, path))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_entries(dict: &str, query: &str) -> Option<DictEntriesResult> {
    NOTIFY_DICT_ENTRIES.get().and_then(|f| f(dict, query))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_dict_entry_write(dict: &str, word: &str, code: &str, commits: i32) -> Option<i32> {
    NOTIFY_DICT_ENTRY_WRITE
        .get()
        .and_then(|f| f(dict, word, code, commits))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_schema_entries(schema_id: &str, query: &str) -> Option<SchemaEntriesResult> {
    NOTIFY_SCHEMA_ENTRIES
        .get()
        .and_then(|f| f(schema_id, query))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_phrase_list(schema_id: &str) -> Option<PhraseListResult> {
    NOTIFY_PHRASE_LIST.get().and_then(|f| f(schema_id))
}

#[cfg(any(windows, feature = "dict-page"))]
fn notify_phrase_save(schema_id: &str, entries: &[CustomPhraseRow]) -> Option<PhraseSaveResult> {
    NOTIFY_PHRASE_SAVE.get().and_then(|f| f(schema_id, entries))
}

fn notify_daemon_reload_plugins() {
    if let Some(f) = NOTIFY_RELOAD_PLUGINS.get() {
        f();
    }
}

fn notify_daemon_reload() -> bool {
    if let Some(f) = NOTIFY_DEPLOY.get() {
        f();
        true
    } else {
        false
    }
}

fn notify_daemon_reload_style() {
    if let Some(f) = NOTIFY_RELOAD_STYLE.get() {
        f();
    }
}

fn notify_select_schema(schema_id: &str) -> bool {
    if let Some(f) = NOTIFY_SELECT_SCHEMA.get() {
        f(schema_id)
    } else {
        false
    }
}

/// 方案市场包目录：数据根下 `market\`（对齐 DECISIONS「下载数据目录映射」
/// 与 server 侧 SchemaManager 同一目录；注册表 .registry.yaml 在数据根）。
pub(crate) fn market_dir() -> std::path::PathBuf {
    let (_, user_data_dir) = get_data_dirs();
    user_data_dir
        .parent()
        .map(|p| p.join("market"))
        .unwrap_or_else(|| {
            let base = std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            base.join(xime_config::app_metadata().config_dir_name)
                .join("market")
        })
}

/// 设置应用的 UI 消息（iced）。
#[derive(Debug, Clone)]
pub enum Message {
    /// 切换左侧导航页面。
    PageSelected(usize),
    /// 输入方案页：切换「已安装 / 已下载」标签。
    SchemaTab(usize),
    /// 输入方案页：选择某个已安装方案。
    SelectSchema(usize),
    /// 部署方案（输入方案页 / 快捷键页 / 方案市场）。
    DeploySchemas,
    /// 安装方案（已下载包 / 方案市场）。
    InstallSchema(String),
    /// 卸载方案。
    UninstallSchema(String),
    /// 安装方案：确认「先卸载冲突方案包再安装」（对齐安卓 confirmInstallWithUninstall）。
    ConfirmSchemaInstall,
    /// 安装方案：取消冲突确认。
    CancelSchemaInstall,
    /// 已安装列表：从 market/builtin/ 备份还原内置方案包。
    RestoreBuiltinSchema,
    /// 方案市场：下载方案。
    DownloadSchema(String),
    /// 扩展商店：下载模型。
    DownloadModel(String),
    /// 扩展商店：删除本地已下载的模型。
    DeleteModel(String),
    /// 输入方案：删除本地已下载的方案包（已安装时必须先卸载）。
    DeleteSchema(String),
    /// 输入方案：打开「导入本地方案」弹窗。
    SchemaImportOpen,
    /// 输入方案：关闭导入弹窗。
    SchemaImportClose,
    /// 输入方案：导入弹窗本地路径输入。
    SchemaImportPathChanged(String),
    /// 输入方案：导入弹窗远程 URL 输入。
    SchemaImportUrlChanged(String),
    /// 输入方案：导入弹窗包 ID 输入。
    SchemaImportPackageIdChanged(String),
    /// 输入方案：浏览本地方案包。
    SchemaImportBrowse,
    /// 输入方案：从剪贴板读取文件/URL。
    SchemaImportClipboard,
    /// 输入方案：开始导入。
    SchemaImportStart,
    /// 输入方案：拖入文件。
    SchemaImportDropped(std::path::PathBuf),
    /// iced 窗口事件（只用于文件拖放）。
    WindowEvent(iced::window::Event),
    /// 输入方案：清理内置方案包（写 marker，server 不再注入 seed）。
    CleanBuiltinSchema,
    /// 输入方案：确认清理内置方案包。
    ConfirmCleanBuiltin,
    /// 输入方案：取消清理内置方案包。
    CancelCleanBuiltin,
    /// 扩展商店：下载插件包。
    DownloadPlugin(String),
    /// 扩展商店：安装已下载的插件包。
    InstallPlugin(String),
    /// 扩展商店：卸载插件。
    UninstallPlugin(String),
    /// 插件管理：确认卸载（二次确认状态）。
    ConfirmUninstallPlugin(String),
    /// 插件管理：刷新已安装插件列表。
    RefreshPlugins,
    /// 插件管理：取消卸载确认。
    CancelUninstallPlugin,
    /// 扩展商店：启用 / 禁用插件。
    TogglePlugin(String, bool),
    /// 扩展商店：方案市场 / 模型市场加载失败后重试。
    MarketRetry,
    /// 输入方案页：打开用户数据目录。
    OpenUserDataDir,
    /// 扩展商店：切换「方案 / 模型」Tab。
    StoreTab(usize),
    /// 扩展商店：分类筛选（"" 表示全部）。
    StoreTagSelected(String),
    /// 扩展商店：选择方案版本。
    SchemaVersionSelected(String, String),
    /// 扩展商店：选择模型版本。
    ModelVersionSelected(String, String),
    /// 外观：字号变更。
    FontSizeChanged(f64),
    /// 外观：候选词数量变更。
    CandidateCountChanged(i32),
    /// 外观：圆角大小变更。
    CornerRadiusChanged(f64),
    /// 外观：浅色模式配色方案变更。
    ColorSchemeLightChanged(String),
    /// 外观：深色模式配色方案变更。
    ColorSchemeDarkChanged(String),
    /// 外观：深色模式变更（0=浅色, 1=深色, 2=跟随系统）。
    DarkModeChanged(u8),
    /// 外观：保存。
    SaveAppearance,
    /// 配置：恢复为客户端默认（删除用户配置文件）。
    RestoreConfigDefaults,
    /// 配置：撤销上一次修改（`.bak` 快照）。
    UndoConfigChange,
    /// 配置：恢复为用户最初提供的版本（`.orig` 快照）。
    RestoreConfigOriginal,
    #[cfg(feature = "smart-suggestion-page")]
    SaveSmartSuggestion,
    #[cfg(feature = "clipboard-page")]
    ClearClipboardHistory,
    /// 剪贴板历史：刷新（重读 server 持久化文件）。
    #[cfg(feature = "clipboard-page")]
    ClipboardHistoryRefresh,
    /// 剪贴板页：切换 Tab（0=历史 1=快捷发送 2=同步）。
    #[cfg(feature = "clipboard-page")]
    ClipboardTab(usize),
    /// 剪贴板历史：点击卡片选中/取消选中。
    #[cfg(feature = "clipboard-page")]
    ClipboardHistorySelected(i64),
    /// 剪贴板历史：删除单条。
    #[cfg(feature = "clipboard-page")]
    ClipboardHistoryRemove(i64),
    /// 剪贴板历史：将条目添加为快捷发送。
    #[cfg(feature = "clipboard-page")]
    QuickSendFromHistory(i64),
    /// 快捷发送：草稿触发编码变更。
    #[cfg(feature = "clipboard-page")]
    QuickSendCodeChanged(String),
    /// 快捷发送：草稿内容变更。
    #[cfg(feature = "clipboard-page")]
    QuickSendContentChanged(String),
    /// 快捷发送：打开新增弹窗。
    #[cfg(feature = "clipboard-page")]
    QuickSendOpen,
    /// 快捷发送：取消新增弹窗。
    #[cfg(feature = "clipboard-page")]
    QuickSendCancel,
    /// 快捷发送：确认添加（弹窗内）。
    #[cfg(feature = "clipboard-page")]
    QuickSendAdd,
    /// 快捷发送：点击卡片选中/取消选中。
    #[cfg(feature = "clipboard-page")]
    QuickSendSelected(i64),
    /// 快捷发送：删除条目（SQLite id）。
    #[cfg(feature = "clipboard-page")]
    QuickSendRemove(i64),
    /// 剪贴板历史：翻页（上一页 / 下一页）。
    #[cfg(feature = "clipboard-page")]
    ClipboardHistoryPrevPage,
    #[cfg(feature = "clipboard-page")]
    ClipboardHistoryNextPage,
    /// 快捷发送：翻页（上一页 / 下一页）。
    #[cfg(feature = "clipboard-page")]
    QuickSendPrevPage,
    #[cfg(feature = "clipboard-page")]
    QuickSendNextPage,
    #[cfg(feature = "clipboard-page")]
    ServerStart,
    #[cfg(feature = "clipboard-page")]
    ServerStop,
    #[cfg(feature = "clipboard-page")]
    ServerRestart,
    #[cfg(feature = "clipboard-page")]
    ServerAddrChanged(String),
    #[cfg(feature = "clipboard-page")]
    ServerUsernameChanged(String),
    #[cfg(feature = "clipboard-page")]
    ServerPasswordChanged(String),
    #[cfg(feature = "clipboard-page")]
    OpenSyncDataDir,
    /// 剪贴板同步插件：启用开关。
    #[cfg(feature = "clipboard-page")]
    SyncPluginEnabled(bool),
    /// 剪贴板同步插件：选择插件。
    #[cfg(feature = "clipboard-page")]
    SyncPluginSelected(usize),
    /// 剪贴板同步插件：编辑配置字段。
    #[cfg(feature = "clipboard-page")]
    SyncPluginFieldChanged(String, String),
    /// 剪贴板同步插件：测试连接。
    #[cfg(feature = "clipboard-page")]
    SyncPluginTest,
    #[cfg(feature = "backup-page")]
    BackupUrlChanged(String),
    #[cfg(feature = "backup-page")]
    BackupUsernameChanged(String),
    #[cfg(feature = "backup-page")]
    BackupPasswordChanged(String),
    #[cfg(feature = "backup-page")]
    BackupDirChanged(String),
    /// 云备份：切换提供者（内置 WebDAV / backup 插件）。
    #[cfg(feature = "backup-page")]
    BackupProviderChanged(usize),
    /// 云备份：编辑插件配置字段（key, value）。
    #[cfg(feature = "backup-page")]
    BackupFieldChanged(String, String),
    /// 云备份：切换备份模式（0=仅配置, 1=全量）。
    #[cfg(feature = "backup-page")]
    BackupModeChanged(u8),
    /// 云备份：测试 WebDAV 连接。
    #[cfg(feature = "backup-page")]
    BackupTest,
    /// 云备份：立即备份。
    #[cfg(feature = "backup-page")]
    BackupNow,
    /// 云备份：查看远端备份列表。
    #[cfg(feature = "backup-page")]
    BackupList,
    /// 云备份：恢复指定备份（远端路径）。
    #[cfg(feature = "backup-page")]
    BackupRestore(String),
    /// 云备份：删除指定备份（远端路径）。
    #[cfg(feature = "backup-page")]
    BackupDelete(String),
    /// 用户资料同步：立即同步（rime sync_user_data，词典快照导出+合并）。
    #[cfg(feature = "backup-page")]
    RimeSyncNow,
    /// 语音转文本：开始/停止听写。
    #[cfg(feature = "voice-page")]
    #[cfg(windows)]
    SpeechToggle,
    /// 语音转文本：清空识别文本。
    #[cfg(feature = "voice-page")]
    #[cfg(windows)]
    SpeechClear,
    /// 语音转文本：复制识别文本到剪贴板。
    #[cfg(feature = "voice-page")]
    #[cfg(windows)]
    SpeechCopy,
    /// 本地模型：下载（模型 id）。
    #[cfg(feature = "voice-page")]
    SpeechModelDownload(String),
    /// 本地模型：删除目录（模型 id）。
    #[cfg(feature = "voice-page")]
    SpeechModelDelete(String),
    /// 本地模型：切换当前使用的模型（模型 id）。
    #[cfg(feature = "voice-page")]
    SpeechModelSelect(String),
    /// 本地模型：试听起停（结果只在本页显示，不上屏）。
    #[cfg(feature = "voice-page")]
    SpeechPreviewToggle,
    /// 本地模型：复制试听文本到剪贴板。
    #[cfg(feature = "voice-page")]
    SpeechModelCopy,
    /// 词典管理：刷新（重读词典列表，选中词典不变）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictRefresh,
    /// 词典管理：备份词典快照（词典名）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictBackup(String),
    /// 词典管理：从快照文件恢复（弹文件对话框）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictRestore,
    /// 词典管理：导出词典为文本（词典名，弹保存对话框）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictExport(String),
    /// 词典管理：从文本导入词典（词典名，弹文件对话框）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictImport(String),
    /// 词典管理：切换页内 Tab（0=用户词典 1=快捷短语）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictTab(usize),
    /// 词典管理：下拉切换当前浏览的词典（立即重读该词典词条）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictSelect(String),
    /// 词典管理：词条搜索关键词变化。
    #[cfg(any(windows, feature = "dict-page"))]
    DictQueryChanged(String),
    /// 词典管理：词条列表翻页。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntriesPage(usize),
    /// 词典管理：打开新增词条对话框。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddOpen,
    /// 词典管理：关闭新增词条对话框（放弃草稿）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddCancel,
    /// 词典管理：新增词条对话框——词。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddWordChanged(String),
    /// 词典管理：新增词条对话框——编码。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddCodeChanged(String),
    /// 词典管理：新增词条对话框——频率（空串 = 1）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddCommitsChanged(String),
    /// 词典管理：提交新增词条。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryAddSubmit,
    /// 词典管理：请求删除词条（进入两步确认，词 + 编码）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryDeleteRequest(String, String),
    /// 词典管理：取消删除。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryDeleteCancel,
    /// 词典管理：确认删除词条（写删除标记）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictEntryDeleteConfirm(String, String),
    /// 快捷短语：切换方案（下拉选择，方案 id）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseSchemaChanged(String),
    /// 快捷短语：打开新增/编辑对话框。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseAddOpen,
    /// 快捷短语：编辑第 i 条（打开对话框并预填）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseEdit(usize),
    /// 快捷短语：关闭对话框（放弃草稿）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDialogCancel,
    /// 快捷短语：对话框——词。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDialogWordChanged(String),
    /// 快捷短语：对话框——编码。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDialogCodeChanged(String),
    /// 快捷短语：对话框——权重（空串 = 省略该列）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDialogWeightChanged(String),
    /// 快捷短语：提交对话框（新增或保存编辑）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDialogSubmit,
    /// 快捷短语：请求删除第 i 条（两步确认）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDeleteRequest(usize),
    /// 快捷短语：取消删除。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDeleteCancel,
    /// 快捷短语：确认删除第 i 条（整表保存）。
    #[cfg(any(windows, feature = "dict-page"))]
    DictPhraseDeleteConfirm(usize),
    /// 方案词表：搜索关键词变化。
    #[cfg(any(windows, feature = "dict-page"))]
    SchemaDictQueryChanged(String),
    /// 方案词表：重新读取当前方案。
    #[cfg(any(windows, feature = "dict-page"))]
    SchemaDictRefresh,
    /// 方案词表：词条列表翻页。
    #[cfg(any(windows, feature = "dict-page"))]
    SchemaDictPage(usize),
    #[cfg(feature = "pair-page")]
    StartPairing,
    /// 订阅轮询：后台任务结果。
    BackgroundPoll,
}
#[derive(Clone)]
pub struct SettingsState {
    pub appearance: AppearanceState,
    pub input_schema: InputSchemaState,
    pub system_theme: SystemTheme,
    pub schemas_loaded: bool,
    pub market_schema: MarketSchemaState,
    /// 输入方案页「导入本地方案」弹窗状态。
    pub schema_import: SchemaImportState,
    pub market_model: MarketModelState,
    pub market_plugin: MarketPluginState,
    /// 插件管理：等待确认卸载的插件 id（None=无）。
    pub plugin_uninstall_confirm: Option<String>,
    pub current_page: usize,
    #[cfg(feature = "smart-suggestion-page")]
    pub smart_suggestion: SmartSuggestionState,
    #[cfg(feature = "pair-page")]
    pub pair: PairState,
    #[cfg(feature = "clipboard-page")]
    pub clipboard: ClipboardState,
    #[cfg(feature = "clipboard-page")]
    pub sync_plugin: SyncPluginUiState,
    /// 剪贴板历史（clipboard_history.json，server 持久化、设置页展示/清空）。
    #[cfg(feature = "clipboard-page")]
    pub clip_history: ClipboardHistoryState,
    /// 快捷发送（quick_send.yaml，设置页编辑，输入法面板后续消费同一文件）。
    #[cfg(feature = "clipboard-page")]
    pub quick_send: QuickSendState,
    /// 剪贴板页当前 Tab（0=历史 1=快捷发送 2=同步）。
    #[cfg(feature = "clipboard-page")]
    pub clipboard_tab: usize,
    #[cfg(feature = "backup-page")]
    pub backup: BackupState,
    /// rime 用户资料同步（快照目录概况 + 立即同步入口）。
    #[cfg(feature = "backup-page")]
    pub rime_sync: RimeSyncState,
    /// 语音转文本（Windows WinRT 听写；worker 归线程所有，此处只存句柄与镜像）。
    #[cfg(feature = "voice-page")]
    #[cfg(windows)]
    pub speech: SpeechState,
    /// 本地离线模型（server 侧引擎）的镜像：模型列表 / 下载进度 / 试听。
    #[cfg(feature = "voice-page")]
    pub speech_server: crate::speech_models::SpeechModelState,
    /// 词典管理（用户词典列表 + 备份/恢复/导出/导入）。
    #[cfg(any(windows, feature = "dict-page"))]
    pub dict_manage: DictManageState,
    /// 全局页内消息条（show_message 写入，5 秒自动过期）。
    pub ui_message: Option<(String, std::time::Instant)>,
    #[cfg(target_os = "linux")]
    pub sync: SyncState,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsState {
    pub fn new() -> Self {
        let mut state = Self {
            appearance: AppearanceState::default(),
            input_schema: InputSchemaState::default(),
            market_schema: MarketSchemaState::default(),
            schema_import: SchemaImportState::default(),
            market_model: MarketModelState::default(),
            market_plugin: MarketPluginState::default(),
            plugin_uninstall_confirm: None,
            system_theme: SystemTheme::detect(),
            schemas_loaded: false,
            current_page: 0,
            #[cfg(feature = "smart-suggestion-page")]
            smart_suggestion: SmartSuggestionState::default(),
            #[cfg(feature = "pair-page")]
            pair: PairState::default(),
            #[cfg(feature = "clipboard-page")]
            clipboard: ClipboardState::default(),
            #[cfg(feature = "clipboard-page")]
            sync_plugin: SyncPluginUiState::default(),
            #[cfg(feature = "clipboard-page")]
            clip_history: ClipboardHistoryState::load(),
            #[cfg(feature = "clipboard-page")]
            quick_send: QuickSendState::load(),
            #[cfg(feature = "clipboard-page")]
            clipboard_tab: 0,
            #[cfg(feature = "backup-page")]
            backup: BackupState::default(),
            #[cfg(feature = "backup-page")]
            rime_sync: RimeSyncState::load(),
            #[cfg(all(feature = "voice-page", windows))]
            speech: SpeechState::default(),
            #[cfg(feature = "voice-page")]
            speech_server: crate::speech_models::SpeechModelState::default(),
            #[cfg(any(windows, feature = "dict-page"))]
            dict_manage: DictManageState::default(),
            ui_message: None,
            #[cfg(target_os = "linux")]
            sync: SyncState::default(),
        };
        state.load_color_schemes();
        state.load_schemas();
        state.load_schema_config();
        state.refresh_installed_plugins();
        state.start_load_market();
        state.start_load_models();
        state.start_load_plugins();
        state
    }

    pub fn colors(&self) -> ThemeColors {
        let primary_color = self.get_primary_color();
        ThemeColors::from_theme(&self.system_theme, primary_color)
    }

    fn get_primary_color(&self) -> u32 {
        let is_dark = self
            .appearance
            .dark_mode
            .is_dark(self.system_theme.is_dark());
        let scheme_name = self.appearance.color_scheme.scheme_name(is_dark);
        self.appearance
            .available_color_schemes
            .iter()
            .find(|(id, _, _)| *id == scheme_name)
            .map(|(_, _, color)| *color)
            .unwrap_or(0x8F73E2)
    }

    pub fn load_schemas(&mut self) {
        if self.schemas_loaded {
            return;
        }
        self.reload_schemas();
    }

    /// 强制重载方案列表（启动时用；安装/卸载后的刷新走后台快照）。
    pub fn reload_schemas(&mut self) {
        self.apply_schema_snapshot(load_schema_snapshot());
    }

    fn apply_schema_snapshot(&mut self, snapshot: SchemaSnapshot) {
        self.input_schema.available_schemas = snapshot.schemas;
        // 「当前方案」是**下标**，而列表按显示名排序（Emoji 这类 ASCII 名永远排最前）。
        // 首次加载/重载时按服务端实际方案反查一次；查不到（服务没起/回调未注册）
        // 才保留原下标，绝不让「当前」标记与引擎各说各话。
        if let Some(current) = notify_current_schema() {
            if let Some(index) = self
                .input_schema
                .available_schemas
                .iter()
                .position(|s| s.schema_id == current)
            {
                self.input_schema.selected_schema = index;
            }
        }
        self.input_schema.schema_packages = snapshot.packages;
        self.input_schema.builtin_restorable = snapshot.builtin_restorable;
        self.input_schema.deployed_schema_ids = snapshot.deployed;
        self.schemas_loaded = true;
    }

    /// 回收后台方案快照（安装/卸载/还原完成后的下一次轮询）。
    fn poll_schema_reload(&mut self) -> bool {
        let snapshot = match schema_snapshot().lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        match snapshot {
            Some(snapshot) => {
                self.apply_schema_snapshot(snapshot);
                true
            }
            None => false,
        }
    }

    pub fn load_schema_config(&mut self) {
        if self.input_schema.config_loaded {
            return;
        }
        if self.input_schema.selected_schema >= self.input_schema.available_schemas.len() {
            return;
        }
        let schema_id =
            &self.input_schema.available_schemas[self.input_schema.selected_schema].schema_id;
        if let Ok(manager) = SchemaConfigManager::new(schema_id) {
            self.input_schema.schema_config = manager.get_config();
            self.input_schema.config_loaded = true;
        }
    }

    pub fn load_color_schemes(&mut self) {
        if self.appearance.color_schemes_loaded {
            return;
        }
        let config = XimeConfig::load();
        self.appearance.color_scheme = config.style.color_scheme.clone();
        self.appearance.dark_mode = config.style.dark_mode;
        self.appearance.available_color_schemes = config
            .color_schemes
            .iter()
            .map(|(id, scheme)| (id.clone(), scheme.name.clone(), scheme.primary_color))
            .collect();
        self.appearance.font_size = config.style.font_size as f64;
        self.appearance.candidate_count = config.style.candidate_count;
        self.appearance.corner_radius = config.style.corner_radius as f64;
        self.appearance.color_schemes_loaded = true;
    }

    pub fn save_color_scheme(&self) -> Result<(), String> {
        let mut config = XimeConfig::load();
        config.style.color_scheme = self.appearance.color_scheme.clone();
        config.style.dark_mode = self.appearance.dark_mode;
        config.save()?;
        notify_daemon_reload_style();
        Ok(())
    }

    pub fn save_appearance(&self) -> Result<(), String> {
        let mut config = XimeConfig::load();
        // 候选数量要落成 rime 的 `menu/page_size`，**必须重新部署**才生效；
        // 字号/配色/圆角靠配置缓存（用户文件 mtime）即时生效，不用部署。
        let count_changed = config.style.candidate_count != self.appearance.candidate_count;
        config.style.font_size = self.appearance.font_size as f32;
        config.style.candidate_count = self.appearance.candidate_count;
        config.style.corner_radius = self.appearance.corner_radius as f32;
        // 配色也并在这一次写入里：分两次 save 会留两份快照，
        // 「撤销上一次修改」就只能退掉一半（改字号撤销后配色还是新的）。
        config.style.color_scheme = self.appearance.color_scheme.clone();
        config.style.dark_mode = self.appearance.dark_mode;
        config.save()?;
        if count_changed {
            notify_daemon_reload();
        } else {
            notify_daemon_reload_style();
        }
        Ok(())
    }

    /// 切换当前输入方案，返回给 UI 的提示语。
    ///
    /// 顺序很关键：**先把「选中方案置顶」的启用列表落盘，再通知宿主**。
    /// 宿主的 `SelectSchema` 只接受已有 build 产物的方案（避免死会话），而产物
    /// 只可能来自启用列表——列表不落盘，兜底部署也编不出这个方案。
    /// 目标没产物时不再同步部署（数秒，会冻住设置窗口），改成后台任务。
    /// 配置文件的快照状态（页面据此决定哪几个按钮可点）。
    pub fn config_file_state() -> xime_config::ConfigFileState {
        XimeConfig::file_state()
    }

    /// 恢复默认：删掉用户配置文件，回到内置 + 系统级。
    pub fn restore_config_defaults(&mut self) -> Result<String, String> {
        XimeConfig::restore_defaults()?;
        Ok(self.after_config_restore("已恢复默认配置（已删除用户配置文件）"))
    }

    /// 撤销上一次修改：用保存时留下的 `.bak` 快照覆盖回去。
    pub fn undo_config_change(&mut self) -> Result<String, String> {
        XimeConfig::undo_last_save()?;
        Ok(self.after_config_restore("已撤销上一次修改"))
    }

    /// 恢复为用户最初提供的版本（程序第一次改写前的那份文件）。
    pub fn restore_config_original(&mut self) -> Result<String, String> {
        XimeConfig::restore_original()?;
        Ok(self.after_config_restore("已恢复为用户提供的配置"))
    }

    /// 恢复/撤销之后：把界面字段重读一遍，并让宿主重载样式。
    fn after_config_restore(&mut self, message: &str) -> String {
        self.appearance.color_schemes_loaded = false;
        self.load_color_schemes();
        notify_daemon_reload_style();
        message.to_string()
    }

    pub fn save_schema(&self) -> Result<String, String> {
        if self.input_schema.selected_schema >= self.input_schema.available_schemas.len() {
            return Ok(String::new());
        }
        let selected = &self.input_schema.available_schemas[self.input_schema.selected_schema];
        let selected_id = selected.schema_id.clone();
        let selected_name = selected.name.clone();

        // 1) 启用列表：选中方案置顶，其余保持原顺序（只加不丢；指向已删除方案的
        //    死项——比如卸载掉的第三方方案——顺手清掉，否则 rime 会报找不到方案）。
        let manager = SchemaManager::new()?;
        let known: Vec<String> = self
            .input_schema
            .available_schemas
            .iter()
            .map(|s| s.schema_id.clone())
            .collect();
        let mut ids: Vec<String> = manager.get_schema_list_ids();
        if ids.is_empty() {
            ids = known.clone();
        }
        ids.retain(|id| known.iter().any(|k| k == id));
        ids.retain(|id| id != &selected_id);
        ids.insert(0, selected_id.clone());
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        manager.set_schema_list(&refs)?;
        manager.save()?;

        // 2) 已部署 → 直接切（毫秒级）。
        if notify_select_schema(&selected_id) {
            return Ok(format!("已切换到「{}」", selected_name));
        }

        // 3) 未部署 → 后台部署 + 让宿主重建会话 + 补选中，结果由 poll_deploy 提示。
        if start_deploy_then_select(&selected_id, &selected_name) {
            Ok(format!(
                "正在切换「{}」：该方案还没有部署产物，正在后台部署…",
                selected_name
            ))
        } else {
            Ok(format!(
                "「{}」还没有部署产物，但已有部署任务在跑，请稍后再点一次",
                selected_name
            ))
        }
    }

    pub fn save_schema_config(&self) -> Result<(), String> {
        if self.input_schema.selected_schema >= self.input_schema.available_schemas.len() {
            return Ok(());
        }

        let schema_id =
            &self.input_schema.available_schemas[self.input_schema.selected_schema].schema_id;
        let manager = SchemaConfigManager::new(schema_id)?;

        let config = &self.input_schema.schema_config;

        if let Some(v) = config.speller.max_code_length {
            manager.set_int("speller/max_code_length", v)?;
        }
        if let Some(v) = config.speller.auto_select {
            manager.set_bool("speller/auto_select", v)?;
        }
        if let Some(v) = &config.speller.auto_clear {
            if !v.is_empty() {
                manager.set_string("speller/auto_clear", v)?;
            }
        }

        if let Some(v) = config.translator.enable_charset_filter {
            manager.set_bool("translator/enable_charset_filter", v)?;
        }
        if let Some(v) = config.translator.enable_completion {
            manager.set_bool("translator/enable_completion", v)?;
        }
        if let Some(v) = config.translator.enable_sentence {
            manager.set_bool("translator/enable_sentence", v)?;
        }
        if let Some(v) = config.translator.enable_user_dict {
            manager.set_bool("translator/enable_user_dict", v)?;
        }
        if let Some(v) = config.translator.enable_encoder {
            manager.set_bool("translator/enable_encoder", v)?;
        }
        if let Some(v) = config.translator.encode_commit_history {
            manager.set_bool("translator/encode_commit_history", v)?;
        }
        if let Some(v) = config.translator.max_phrase_length {
            manager.set_int("translator/max_phrase_length", v)?;
        }

        if let Some(v) = &config.reverse_lookup.prefix {
            manager.set_string("reverse_lookup/prefix", v)?;
        }
        if let Some(v) = &config.reverse_lookup.suffix {
            manager.set_string("reverse_lookup/suffix", v)?;
        }

        if let Some(v) = &config.tradition.opencc_config {
            manager.set_string("tradition/opencc_config", v)?;
        }

        manager.save()?;

        Ok(())
    }

    // ---- 部署（后台线程执行，结果经轮询回收） ----

    /// 显示结果消息：触发系统通知回调（若宿主注册了 `set_notify_message`）。
    pub fn show_message(&mut self, msg: String) {
        // 页内消息条（BackgroundPoll 5 秒后过期清理）。
        self.ui_message = Some((msg.clone(), std::time::Instant::now()));
        if let Some(f) = NOTIFY_MESSAGE.get() {
            f(xime_config::app_metadata().display_name, &msg);
        }
    }

    /// 页内消息条是否仍有效。
    pub fn ui_message(&self) -> Option<&str> {
        let (msg, at) = self.ui_message.as_ref()?;
        (at.elapsed() < std::time::Duration::from_secs(5)).then_some(msg.as_str())
    }

    pub fn start_deploy(&mut self) {
        if deploy_result().lock().unwrap().is_some() {
            return;
        }
        self.show_message("正在部署…".to_string());
        std::thread::spawn(|| {
            // 部署 + daemon 重载都在后台线程：server 的 redeploy 可能数秒，
            // IPC 同步等待绝不能进 UI 线程（否则设置窗口整个冻结）。
            let result = (|| -> Result<String, String> {
                deploy_all().map_err(|e| e.to_string())?;
                if notify_daemon_reload() {
                    Ok("部署成功！配置已重载。".to_string())
                } else {
                    Ok("部署成功！(服务器未运行，配置将在下次启动时生效)".to_string())
                }
            })();
            match &result {
                Ok(msg) => notify_deploy_toast("方案部署完成", msg),
                Err(e) => notify_deploy_toast("方案部署失败", e),
            }
            *deploy_result().lock().unwrap() = Some(result);
        });
    }

    pub fn poll_deploy(&mut self) {
        let result = deploy_result().lock().unwrap().take();
        if let Some(result) = result {
            match result {
                Ok(msg) => self.show_message(msg),
                Err(e) => self.show_message(format!("部署失败: {}", e)),
            }
        }
    }

    // ---- 扩展商店：方案市场 ----

    pub fn start_load_market(&mut self) {
        if self.market_schema.loaded || self.market_schema.loading {
            return;
        }
        self.market_schema.loading = true;

        std::thread::spawn(|| {
            let result = (|| -> Result<String, String> {
                let response = http_client()
                    .get("https://index.ximei.me/rimes/index.yaml")
                    .send()
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| format!("网络请求失败: {}", e))?;
                response.text().map_err(|e| format!("读取响应失败: {}", e))
            })();

            *market_yaml_result().lock().unwrap() = Some(result);
        });
    }

    /// 轮询方案索引结果。返回是否有变化。
    pub fn poll_market_yaml(&mut self) -> bool {
        if !self.market_schema.loading {
            return false;
        }
        let result = market_yaml_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            Ok(text) => match serde_yaml::from_str::<SchemaIndex>(&text) {
                Ok(index) => {
                    self.market_schema.installed_ids = self.get_installed_package_ids();
                    self.market_schema.downloaded_ids = self.get_cached_schema_ids();
                    self.market_schema.schemas = index.schemas;
                    self.market_schema.updated_at = index.updated_at;
                    self.market_schema.loaded = true;
                    self.market_schema.loading = false;
                    self.market_schema.error = None;
                }
                Err(e) => {
                    self.market_schema.loading = false;
                    self.market_schema.error = Some(format!("解析失败: {}", e));
                    tracing::warn!("方案索引解析失败: {e}");
                }
            },
            Err(e) => {
                self.market_schema.loading = false;
                tracing::warn!("方案索引拉取失败: {e}");
                self.market_schema.error = Some(e);
            }
        }
        true
    }

    /// 轮询方案市场后台任务结果。返回是否有变化。
    pub fn poll_market_task(&mut self) -> bool {
        let result = market_task_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            MarketTaskResult::DownloadDone(id) => {
                if !self.market_schema.downloaded_ids.contains(&id) {
                    self.market_schema.downloaded_ids.push(id);
                }
            }
            MarketTaskResult::InstallDone => {
                // 已安装包列表按注册表重建（安装/卸载后的归属即事实源），
                // 已安装方案列表立即刷新（此前只在启动时加载一次，装完不变）。
                self.market_schema.installed_ids = self.get_installed_package_ids();
                start_schema_reload_async();
            }
            MarketTaskResult::UninstallDone => {
                self.market_schema.installed_ids = self.get_installed_package_ids();
                start_schema_reload_async();
            }
            MarketTaskResult::DeleteDone(id) => {
                self.market_schema.downloaded_ids.retain(|i| i != &id);
            }
            MarketTaskResult::BuiltinRestored(n) => {
                self.market_schema.installed_ids = self.get_installed_package_ids();
                start_schema_reload_async();
                self.market_schema.install_message =
                    Some(format!("已还原 {n} 个内置方案文件，默认方案已启用"));
                self.market_schema.install_message_since = Some(std::time::Instant::now());
            }
            MarketTaskResult::BuiltinCleaned(n) => {
                self.market_schema.installed_ids = self.get_installed_package_ids();
                start_schema_reload_async();
                self.market_schema.install_message = Some(format!(
                    "已清理 {n} 个内置方案文件；可在「本地方案」或市场方案中继续使用"
                ));
                self.market_schema.install_message_since = Some(std::time::Instant::now());
            }
            MarketTaskResult::Error(e) => {
                self.market_schema.install_message = Some(e);
                self.market_schema.install_message_since = Some(std::time::Instant::now());
            }
        }
        self.market_schema.downloading = None;
        self.market_schema.installing = None;
        true
    }

    /// 轮询扩展商店下载进度（方案 / 模型通用）。返回是否有变化。
    fn poll_download_progress(&mut self) -> bool {
        let result = download_progress().lock().unwrap().take();
        let Some((id, progress)) = result else {
            return false;
        };
        if self.market_schema.downloading.as_deref() == Some(&id) {
            self.market_schema.download_progress = Some(progress);
        }
        if self.market_model.downloading.as_deref() == Some(&id) {
            self.market_model.download_progress = Some(progress);
        }
        true
    }

    // ---- 扩展商店：模型市场 ----

    pub fn start_load_models(&mut self) {
        if self.market_model.loaded || self.market_model.loading {
            return;
        }
        self.market_model.loading = true;

        std::thread::spawn(|| {
            let result = (|| -> Result<String, String> {
                let response = http_client()
                    .get("https://index.ximei.me/models/index.yaml")
                    .send()
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| format!("网络请求失败: {}", e))?;
                response.text().map_err(|e| format!("读取响应失败: {}", e))
            })();

            *model_yaml_result().lock().unwrap() = Some(result);
        });
    }

    /// 轮询模型索引结果。返回是否有变化。
    pub fn poll_model_yaml(&mut self) -> bool {
        if !self.market_model.loading {
            return false;
        }
        let result = model_yaml_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            Ok(text) => match serde_yaml::from_str::<ModelIndex>(&text) {
                Ok(index) => {
                    self.market_model.downloaded_ids = self.get_cached_model_ids();
                    self.market_model.models = index.models;
                    self.market_model.updated_at = index.updated_at;
                    self.market_model.loaded = true;
                    self.market_model.loading = false;
                    self.market_model.error = None;
                }
                Err(e) => {
                    self.market_model.loading = false;
                    self.market_model.error = Some(format!("解析失败: {}", e));
                    tracing::warn!("模型索引解析失败: {e}");
                }
            },
            Err(e) => {
                self.market_model.loading = false;
                tracing::warn!("模型索引拉取失败: {e}");
                self.market_model.error = Some(e);
            }
        }
        true
    }

    /// 轮询模型市场后台任务结果。返回是否有变化。
    pub fn poll_model_task(&mut self) -> bool {
        let result = model_task_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            ModelTaskResult::DownloadDone(id) => {
                if !self.market_model.downloaded_ids.contains(&id) {
                    self.market_model.downloaded_ids.push(id);
                }
            }
            ModelTaskResult::DeleteDone(id) => {
                self.market_model.downloaded_ids.retain(|i| i != &id);
            }
            ModelTaskResult::Error(e) => {
                self.market_model.install_message = Some(e);
                self.market_model.install_message_since = Some(std::time::Instant::now());
            }
        }
        self.market_model.downloading = None;
        self.market_model.download_progress = None;
        true
    }

    /// 统一回收后台任务结果（由轮询订阅调用）。
    pub fn poll_background(&mut self) {
        self.poll_deploy();
        self.poll_market_yaml();
        self.poll_model_yaml();
        self.poll_plugin_yaml();
        self.poll_market_task();
        self.poll_schema_reload();
        self.poll_schema_import();
        self.poll_model_task();
        self.poll_plugin_task();
        self.poll_download_progress();
        self.expire_install_messages();
        #[cfg(feature = "clipboard-page")]
        self.clipboard.poll();
        #[cfg(feature = "clipboard-page")]
        self.sync_plugin.poll();
        #[cfg(feature = "backup-page")]
        self.backup.poll();
        #[cfg(feature = "backup-page")]
        if let Some(result) = self.rime_sync.poll_sync() {
            match result {
                Ok(msg) => self.show_message(msg),
                Err(e) => self.show_message(e),
            }
        }
        #[cfg(any(windows, feature = "dict-page"))]
        self.dict_manage.poll();
        #[cfg(any(windows, feature = "dict-page"))]
        self.poll_schema_dict();
        #[cfg(all(feature = "voice-page", windows))]
        self.speech.poll();
        // 本地模型状态：只在本页可见时真的发 IPC（见 SpeechModelState::poll）。
        #[cfg(feature = "voice-page")]
        self.speech_server.poll();
    }

    // ---- 方案词表（输入方案页「方案词表」tab，只读浏览） ----

    /// 当前选中的方案（id + 显示名，取输入方案页选中行）。
    #[cfg(any(windows, feature = "dict-page"))]
    pub fn selected_schema_info(&self) -> Option<(String, String)> {
        let index = self.input_schema.selected_schema;
        let info = self.input_schema.available_schemas.get(index)?;
        Some((info.schema_id.clone(), info.name.clone()))
    }

    /// 进入「方案词表」tab 或切换选中方案时调用：确保为当前方案读过一次。
    #[cfg(any(windows, feature = "dict-page"))]
    ///
    /// 同一方案且已读过/在途 → 不重读；换方案 → 清状态重读（在途的旧结果会
    /// 被 `poll_schema_dict` 按 schema_id 丢弃）。
    pub fn schema_dict_ensure(&mut self) {
        let Some((schema_id, _)) = self.selected_schema_info() else {
            return;
        };
        let dict = &mut self.input_schema.dict;
        if dict.schema_id == schema_id && (dict.loaded || dict.loading) {
            return;
        }
        dict.schema_id = schema_id;
        dict.entries.clear();
        dict.page = 0;
        dict.loaded = false;
        dict.loading = false;
        dict.error = None;
        self.start_schema_fetch();
    }

    /// 方案词表搜索关键词变化：重置页码，防抖后重读。
    #[cfg(any(windows, feature = "dict-page"))]
    pub fn schema_dict_set_query(&mut self, query: String) {
        let dict = &mut self.input_schema.dict;
        if dict.query == query {
            return;
        }
        dict.query = query;
        dict.page = 0;
        dict.pending = Some(std::time::Instant::now());
    }

    /// 重新读取当前方案的词表。
    #[cfg(any(windows, feature = "dict-page"))]
    pub fn schema_dict_refresh(&mut self) {
        self.start_schema_fetch();
    }

    /// 方案词表翻页（夹取到范围内）。
    #[cfg(any(windows, feature = "dict-page"))]
    pub fn schema_dict_page(&mut self, page: usize) {
        let dict = &mut self.input_schema.dict;
        let pages = dict.page_count();
        dict.page = if pages == 0 { 0 } else { page.min(pages - 1) };
    }

    /// 发起一次方案词表读取（单飞 + 防抖，与用户词典词条读取同款）。
    #[cfg(any(windows, feature = "dict-page"))]
    fn start_schema_fetch(&mut self) {
        let dict = &mut self.input_schema.dict;
        if dict.schema_id.is_empty() {
            return;
        }
        if dict.loading {
            dict.pending = Some(std::time::Instant::now());
            return;
        }
        dict.loading = true;
        dict.pending = None;
        dict.error = None;
        let schema_id = dict.schema_id.clone();
        let query = dict.query.clone();
        std::thread::spawn(move || {
            let outcome = match notify_schema_entries(&schema_id, &query) {
                Some(result) => SchemaDictTaskResult::Entries { schema_id, result },
                None => SchemaDictTaskResult::Failed {
                    schema_id,
                    reason: "读取方案词表失败（输入法服务未运行？）".to_string(),
                },
            };
            *SCHEMA_DICT_OUTCOME
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        });
    }

    /// BackgroundPoll 节拍：方案词表结果回收 + 关键词防抖。
    #[cfg(any(windows, feature = "dict-page"))]
    pub fn poll_schema_dict(&mut self) {
        let outcome = SCHEMA_DICT_OUTCOME
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match outcome {
            Some(SchemaDictTaskResult::Entries { schema_id, result }) => {
                let dict = &mut self.input_schema.dict;
                if dict.schema_id == schema_id {
                    dict.loading = false;
                    dict.loaded = true;
                    dict.total = result.total;
                    dict.matched = result.matched;
                    dict.entries = result.entries;
                    dict.dict_name = result.dict_name;
                    dict.tables = result.tables;
                    dict.missing = result.missing;
                    dict.page = 0;
                }
            }
            Some(SchemaDictTaskResult::Failed { schema_id, reason }) => {
                let dict = &mut self.input_schema.dict;
                if dict.schema_id == schema_id {
                    dict.loading = false;
                    dict.loaded = true;
                    dict.error = Some(reason);
                }
            }
            None => {}
        }
        // 关键词改动过了防抖就补一次读取。
        let due = self
            .input_schema
            .dict
            .pending
            .map(|since| since.elapsed() >= DICT_QUERY_DEBOUNCE)
            .unwrap_or(false);
        if due {
            self.start_schema_fetch();
        }
    }

    /// 扩展商店安装/卸载消息 4 秒后自动消失。
    fn expire_install_messages(&mut self) {
        let now = std::time::Instant::now();
        let expired = std::time::Duration::from_secs(4);
        for (msg, since) in [
            (
                &mut self.market_schema.install_message,
                &mut self.market_schema.install_message_since,
            ),
            (
                &mut self.market_model.install_message,
                &mut self.market_model.install_message_since,
            ),
            (
                &mut self.market_plugin.install_message,
                &mut self.market_plugin.install_message_since,
            ),
        ] {
            if msg.is_some()
                && since
                    .map(|t| now.duration_since(t) > expired)
                    .unwrap_or(true)
            {
                *msg = None;
                *since = None;
            }
        }
    }

    /// 重新加载扩展商店（方案 + 模型索引）。
    pub fn refresh_store(&mut self) {
        self.market_schema.loaded = false;
        self.market_schema.loading = false;
        self.market_schema.error = None;
        self.start_load_market();
        self.market_model.loaded = false;
        self.market_model.loading = false;
        self.market_model.error = None;
        self.start_load_models();
        self.market_plugin.loaded = false;
        self.market_plugin.loading = false;
        self.market_plugin.error = None;
        self.start_load_plugins();
    }

    // ---- 扩展商店：插件市场 ----

    pub fn start_load_plugins(&mut self) {
        if self.market_plugin.loaded || self.market_plugin.loading {
            return;
        }
        self.market_plugin.loading = true;

        std::thread::spawn(|| {
            let result = (|| -> Result<String, String> {
                // v2 子索引：JS/QuickJS 插件（minHostVersion 3.0.0，对齐
                // xime-plugin quickjs 运行时）；根路径 v1 为 Lua 时代遗留索引。
                let response = http_client()
                    .get("https://index.ximei.me/plugins/v2/index.yaml")
                    .send()
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| format!("网络请求失败: {}", e))?;
                response.text().map_err(|e| format!("读取响应失败: {}", e))
            })();

            *plugin_yaml_result().lock().unwrap() = Some(result);
        });
    }

    /// 轮询插件索引结果。返回是否有变化。
    pub fn poll_plugin_yaml(&mut self) -> bool {
        if !self.market_plugin.loading {
            return false;
        }
        let result = plugin_yaml_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            Ok(text) => match serde_yaml::from_str::<PluginIndex>(&text) {
                Ok(index) => {
                    self.market_plugin.installed = self.installed_plugins();
                    self.market_plugin.plugins = index.plugins;
                    self.market_plugin.updated_at = index.updated_at;
                    self.market_plugin.loaded = true;
                    self.market_plugin.loading = false;
                    self.market_plugin.error = None;
                }
                Err(e) => {
                    self.market_plugin.loading = false;
                    self.market_plugin.error = Some(format!("解析失败: {}", e));
                    tracing::warn!("插件索引解析失败: {e}");
                }
            },
            Err(e) => {
                self.market_plugin.loading = false;
                tracing::warn!("插件索引拉取失败: {e}");
                self.market_plugin.error = Some(e);
            }
        }
        true
    }

    /// 轮询插件市场后台任务结果。返回是否有变化。
    pub fn poll_plugin_task(&mut self) -> bool {
        let result = plugin_task_result().lock().unwrap().take();
        let Some(result) = result else {
            return false;
        };
        match result {
            PluginTaskResult::InstallDone(id) => {
                self.market_plugin.installed = self.installed_plugins();
                self.market_plugin.downloaded_ids.retain(|i| i != &id);
                notify_daemon_reload_plugins();
            }
            PluginTaskResult::UninstallDone(id) => {
                self.market_plugin.installed = self.installed_plugins();
                self.market_plugin.downloaded_ids.retain(|i| i != &id);
                notify_daemon_reload_plugins();
            }
            PluginTaskResult::ToggleDone(id, enabled) => {
                if let Some(p) = self.market_plugin.installed.iter_mut().find(|p| p.id == id) {
                    p.enabled = enabled;
                }
                notify_daemon_reload_plugins();
            }
            PluginTaskResult::Error(e) => {
                self.market_plugin.install_message = Some(e);
                self.market_plugin.install_message_since = Some(std::time::Instant::now());
            }
        }
        self.market_plugin.downloading = None;
        self.market_plugin.installing = None;
        true
    }

    pub fn download_market_plugin(&mut self, plugin_id: &str) {
        if self.market_plugin.downloading.is_some() || self.market_plugin.installing.is_some() {
            return;
        }

        let plugin = match self
            .market_plugin
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
        {
            Some(p) => p.clone(),
            None => return,
        };

        self.market_plugin.downloading = Some(plugin_id.to_string());
        self.market_plugin.download_progress = None;
        self.market_plugin.install_message = None;

        // 下载即安装：下载到临时 .xipk 后立即解压注册，成功后删除临时包。
        std::thread::spawn(move || {
            let result = do_download_plugin(&plugin).and_then(|xipk| {
                let install = plugin_manager()
                    .install_from_zip(&xipk, true)
                    .map(|_| ())
                    .map_err(|e| anyhow::anyhow!("{}", e));
                std::fs::remove_file(&xipk).ok();
                install
            });
            let task = match result {
                Ok(()) => PluginTaskResult::InstallDone(plugin.id.clone()),
                Err(e) => PluginTaskResult::Error(e.to_string()),
            };
            *plugin_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn install_market_plugin(&mut self, plugin_id: &str) {
        if self.market_plugin.installing.is_some() || self.market_plugin.downloading.is_some() {
            return;
        }

        self.market_plugin.installing = Some(plugin_id.to_string());
        self.market_plugin.install_message = None;

        let pid = plugin_id.to_string();
        std::thread::spawn(move || {
            let result = do_install_plugin(&pid);
            let task = match result {
                Ok(()) => PluginTaskResult::InstallDone(pid),
                Err(e) => PluginTaskResult::Error(e.to_string()),
            };
            *plugin_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn uninstall_market_plugin(&mut self, plugin_id: &str) {
        let pid = plugin_id.to_string();
        std::thread::spawn(move || {
            let result = plugin_manager().uninstall(&pid).map_err(|e| e.to_string());
            let task = match result {
                Ok(()) => PluginTaskResult::UninstallDone(pid),
                Err(e) => PluginTaskResult::Error(e),
            };
            *plugin_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn toggle_market_plugin(&mut self, plugin_id: &str, enabled: bool) {
        let pid = plugin_id.to_string();
        std::thread::spawn(move || {
            let result = plugin_manager()
                .set_enabled(&pid, enabled)
                .map_err(|e| e.to_string());
            let task = match result {
                Ok(()) => PluginTaskResult::ToggleDone(pid, enabled),
                Err(e) => PluginTaskResult::Error(e),
            };
            *plugin_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn installed_plugins(&self) -> Vec<xime_plugin::PluginRecord> {
        plugin_manager().list()
    }

    /// 插件管理页：重新扫描已安装插件列表。
    pub fn refresh_installed_plugins(&mut self) {
        self.market_plugin.installed = self.installed_plugins();
    }

    pub fn download_market_schema(&mut self, schema_id: &str) {
        if self.market_schema.downloading.is_some() || self.market_schema.installing.is_some() {
            return;
        }

        let schema = match self
            .market_schema
            .schemas
            .iter()
            .find(|s| s.id == schema_id)
        {
            Some(s) => s.clone(),
            None => return,
        };

        self.market_schema.downloading = Some(schema_id.to_string());
        self.market_schema.download_progress = None;
        self.market_schema.install_message = None;

        std::thread::spawn(move || {
            let result = do_download(&schema);
            let task = match result {
                Ok(()) => MarketTaskResult::DownloadDone(schema.id.clone()),
                Err(e) => MarketTaskResult::Error(e.to_string()),
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    /// 安装已下载 / 市场里的方案包。
    ///
    /// 对齐安卓 `SchemaLocalViewModel.installPackage`：rime 目录里已有其他方案包
    /// （含内置方案包）时**不能直接装**，先弹确认「需要先卸载冲突方案包」——
    /// 这就是方案隔离：一次只存在一个方案包，第三方方案不会与内置方案混装。
    pub fn install_market_schema(&mut self, schema_id: &str) {
        if self.market_schema.installing.is_some() || self.market_schema.downloading.is_some() {
            return;
        }

        match schema_install_conflict(schema_id) {
            Ok(conflict) if conflict.packages.is_empty() => self.start_schema_install(schema_id),
            Ok(conflict) => {
                self.market_schema.install_message = None;
                self.market_schema.conflict_install = Some(conflict);
            }
            Err(e) => {
                self.market_schema.install_message = Some(e);
                self.market_schema.install_message_since = Some(std::time::Instant::now());
            }
        }
    }

    /// 确认「先卸载冲突方案包再安装」（对齐安卓 `confirmInstallWithUninstall`）：
    /// 逐个精确卸载冲突包（不单独部署）→ 安装目标包（安装流程内统一部署一次）。
    pub fn confirm_schema_install(&mut self) {
        let Some(conflict) = self.market_schema.conflict_install.take() else {
            return;
        };
        if self.market_schema.installing.is_some() || self.market_schema.downloading.is_some() {
            return;
        }

        self.market_schema.installing = Some(conflict.schema_id.clone());
        self.market_schema.install_message = None;

        std::thread::spawn(move || {
            let sid = conflict.schema_id.clone();
            if conflict.is_restore_builtin() {
                // 方案来源互斥：还原内置方案前先卸载第三方方案包，
                // 否则又会变成内置 + 第三方混装。
                let result = (|| -> anyhow::Result<usize> {
                    for pkg in &conflict.packages {
                        do_uninstall(pkg, false)?;
                    }
                    let manifest = schema_manifest().map_err(|e| anyhow::anyhow!(e))?;
                    let restored = manifest
                        .restore_builtin_package()
                        .map_err(|e| anyhow::anyhow!(e))?;
                    apply_restored_builtin_schema_list(&manifest)?;
                    deploy_all().map_err(|e| anyhow::anyhow!("部署失败: {}", e))?;
                    notify_daemon_reload();
                    Ok(restored)
                })();
                let task = match result {
                    Ok(n) => {
                        notify_deploy_toast(
                            "内置方案已还原",
                            &format!("已卸载第三方方案包，{n} 个内置方案文件已回到数据目录"),
                        );
                        MarketTaskResult::BuiltinRestored(n)
                    }
                    Err(e) => {
                        notify_deploy_toast("内置方案还原失败", &e.to_string());
                        MarketTaskResult::Error(e.to_string())
                    }
                };
                *market_task_result().lock().unwrap() = Some(task);
                return;
            }
            let result = (|| -> anyhow::Result<()> {
                // 先确认新包本身可安装，再卸载旧方案；坏包不能把现有方案删光。
                validate_package_archive(&sid)?;
                for pkg in &conflict.packages {
                    do_uninstall(pkg, false)?;
                }
                do_install(&sid)
            })();
            let task = match result {
                Ok(()) => {
                    notify_deploy_toast(
                        "方案部署完成",
                        &format!("{} 已启用（已先卸载冲突方案包，避免混装）", sid),
                    );
                    MarketTaskResult::InstallDone
                }
                Err(e) => {
                    notify_deploy_toast("方案部署失败", &format!("{sid}：{e}"));
                    MarketTaskResult::Error(e.to_string())
                }
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    /// 取消安装冲突确认（不改动任何文件）。
    pub fn cancel_schema_install(&mut self) {
        self.market_schema.conflict_install = None;
    }

    /// 从 `market/builtin/` 备份还原内置方案包（隔离安装第三方方案后恢复内置方案）。
    /// 方案来源互斥：若 rime 目录里还有第三方方案包，先确认卸载它们再还原，
    /// 不允许还原成「内置 + 第三方」混装状态。
    pub fn restore_builtin_schema(&mut self) {
        if self.market_schema.installing.is_some() || self.market_schema.downloading.is_some() {
            return;
        }
        match schema_restore_conflict() {
            Ok(packages) if packages.is_empty() => self.start_builtin_restore(),
            Ok(packages) => {
                self.market_schema.install_message = None;
                self.market_schema.conflict_install = Some(SchemaInstallConflict {
                    schema_id: BUILTIN_PACKAGE_ID.to_string(),
                    packages,
                });
            }
            Err(e) => {
                self.market_schema.install_message = Some(e);
                self.market_schema.install_message_since = Some(std::time::Instant::now());
            }
        }
    }

    /// 清理内置方案包：删 builtin 文件、写「已清理」标记（server 重启不再注入
    /// seed），并把启用列表里的内置方案 id 去掉。要求至少还有一个非内置方案可用。
    pub fn clean_builtin_schema(&mut self) {
        if self.market_schema.installing.is_some() || self.market_schema.downloading.is_some() {
            return;
        }
        self.market_schema.installing = Some(BUILTIN_PACKAGE_ID.to_string());
        self.market_schema.install_message = None;

        std::thread::spawn(|| {
            let result = (|| -> anyhow::Result<usize> {
                let manifest = schema_manifest().map_err(|e| anyhow::anyhow!(e))?;
                let manager = SchemaManager::new().map_err(|e| anyhow::anyhow!("{}", e))?;
                let registry = manifest.load_registry();
                let builtin_ids: Vec<String> = registry
                    .get(BUILTIN_PACKAGE_ID)
                    .map(|entry| {
                        package_schema_ids(entry.files.iter().map(String::as_str), None)
                    })
                    .unwrap_or_default();
                let local_ids: Vec<String> = registry
                    .get(LOCAL_PACKAGE_ID)
                    .map(|entry| {
                        package_schema_ids(entry.files.iter().map(String::as_str), None)
                    })
                    .unwrap_or_default();
                let mut remaining: Vec<String> = manager
                    .get_schema_list()
                    .into_iter()
                    .map(|s| s.schema_id)
                    .filter(|id| !builtin_ids.iter().any(|b| b == id))
                    .collect();
                if remaining.is_empty() {
                    // 还原内置会把启用列表重置成只有 builtin；清理时用 registry 的
                    // 本地方案再兜底一次。
                    remaining = local_ids;
                }
                if remaining.is_empty() {
                    anyhow::bail!(
                        "没有其他可用方案：请先放入自己的方案（或安装市场方案）并启用，再清理内置"
                    );
                }

                // 先把剩余方案写进启用列表并部署/切换，再删 builtin：
                // 否则 server 的 active schema 还是即将被删掉的内置方案，redeploy
                // 会失败并弹一条「部署失败」通知。
                let target = remaining[0].clone();
                let refs: Vec<&str> = remaining.iter().map(String::as_str).collect();
                manager
                    .set_schema_list(&refs)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                manager.save().map_err(|e| anyhow::anyhow!("{}", e))?;
                deploy_all().map_err(|e| anyhow::anyhow!("部署失败: {}", e))?;
                if !notify_select_schema(&target) {
                    anyhow::bail!("切换到剩余方案「{target}」失败，内置方案未删除");
                }

                let outcome = manifest
                    .uninstall_package(BUILTIN_PACKAGE_ID)
                    .map_err(|e| anyhow::anyhow!(e))?;
                manifest
                    .set_builtin_removed(true)
                    .map_err(|e| anyhow::anyhow!(e))?;

                notify_daemon_reload();
                Ok(outcome.deleted_files)
            })();
            let task = match result {
                Ok(n) => {
                    notify_deploy_toast(
                        "内置方案已清理",
                        &format!("{n} 个内置方案文件已删除；重启输入法后不会自动注入"),
                    );
                    MarketTaskResult::BuiltinCleaned(n)
                }
                Err(e) => {
                    notify_deploy_toast("内置方案清理失败", &e.to_string());
                    MarketTaskResult::Error(e.to_string())
                }
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    /// 后台线程执行内置方案还原（无冲突或已确认卸载第三方包后）。
    fn start_builtin_restore(&mut self) {
        self.market_schema.installing = Some(BUILTIN_PACKAGE_ID.to_string());
        self.market_schema.install_message = None;

        std::thread::spawn(|| {
            let result = (|| -> anyhow::Result<usize> {
                let manifest = schema_manifest().map_err(|e| anyhow::anyhow!(e))?;
                let restored = manifest
                    .restore_builtin_package()
                    .map_err(|e| anyhow::anyhow!(e))?;
                manifest
                    .set_builtin_removed(false)
                    .map_err(|e| anyhow::anyhow!(e))?;
                apply_restored_builtin_schema_list(&manifest)?;
                deploy_all().map_err(|e| anyhow::anyhow!("部署失败: {}", e))?;
                notify_daemon_reload();
                Ok(restored)
            })();
            let task = match result {
                Ok(n) => {
                    notify_deploy_toast(
                        "内置方案已还原",
                        &format!("{n} 个内置方案文件已回到数据目录"),
                    );
                    MarketTaskResult::BuiltinRestored(n)
                }
                Err(e) => {
                    notify_deploy_toast("内置方案还原失败", &e.to_string());
                    MarketTaskResult::Error(e.to_string())
                }
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    /// 后台线程执行安装（无冲突或已确认卸载后）。
    fn start_schema_install(&mut self, schema_id: &str) {
        self.market_schema.installing = Some(schema_id.to_string());
        self.market_schema.install_message = None;

        let sid = schema_id.to_string();
        std::thread::spawn(move || {
            let result = do_install(&sid);
            let task = match result {
                Ok(()) => {
                    notify_deploy_toast("方案部署完成", &format!("{sid} 已启用，可直接输入使用"));
                    MarketTaskResult::InstallDone
                }
                Err(e) => {
                    notify_deploy_toast("方案部署失败", &format!("{sid}：{e}"));
                    MarketTaskResult::Error(e.to_string())
                }
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    /// 输入方案页「导入本地方案」：开始复制/下载到 market 缓存。
    pub fn start_schema_import(&mut self) {
        if self.schema_import.busy {
            return;
        }
        let path = trim_import_input(&self.schema_import.path);
        let url = trim_import_input(&self.schema_import.url);
        let Some(id) = sanitize_package_id(&self.schema_import.package_id) else {
            self.schema_import.message = Some("方案名称不能为空，也不能只由 . - 组成".into());
            return;
        };
        if path.is_empty() == url.is_empty() {
            self.schema_import.message =
                Some("请只选择一种来源：本地文件路径或远程 URL".into());
            return;
        }
        if market_dir().join(&id).exists()
            || self.market_schema.installed_ids.iter().any(|x| x == &id)
        {
            self.schema_import.message =
                Some(format!("方案名称「{id}」已存在，请改名或先卸载/删除"));
            return;
        }

        self.schema_import.busy = true;
        self.schema_import.message = None;
        self.schema_import.progress = None;
        *schema_import_progress().lock().unwrap() = None;
        std::thread::spawn(move || {
            let result = prepare_import(&path, &url, &id);
            *schema_import_result().lock().unwrap() = Some(result);
        });
    }

    /// 回收导入准备结果；准备成功后再走正常安装流程（含冲突确认）。
    fn poll_schema_import(&mut self) -> bool {
        let mut changed = false;
        let picked = match schema_import_browse().lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(picked) = picked {
            self.schema_import.browsing = false;
            if let Some(path) = picked {
                self.schema_import.set_path(path.display().to_string());
            }
            changed = true;
        }
        if self.schema_import.open && self.schema_import.busy {
            self.schema_import.tick = self.schema_import.tick.wrapping_add(1);
            changed = true;
        }
        let progress = match schema_import_progress().lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(progress) = progress {
            self.schema_import.progress = Some(progress);
            return true;
        }
        let result = match schema_import_result().lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        let Some(result) = result else {
            return changed;
        };
        self.schema_import.busy = false;
        self.schema_import.progress = None;
        match result {
            Ok(id) => {
                self.schema_import.open = false;
                if !self.market_schema.downloaded_ids.contains(&id) {
                    self.market_schema.downloaded_ids.push(id.clone());
                }
                self.install_market_schema(&id);
                self.schema_import.message = None;
            }
            Err(e) => self.schema_import.message = Some(e),
        }
        true
    }

    pub fn browse_import_file(&mut self) {
        if self.schema_import.browsing {
            return;
        }
        self.schema_import.browsing = true;
        self.schema_import.message = None;
        std::thread::spawn(|| {
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Com::{
                    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
                };
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let picked = rfd::FileDialog::new()
                    .add_filter("输入方案包", &["zip", "tar.gz"])
                    .pick_file();
                *schema_import_browse().lock().unwrap() = Some(picked);
                CoUninitialize();
            }
            #[cfg(not(windows))]
            {
                let picked = rfd::FileDialog::new()
                    .add_filter("输入方案包", &["zip", "tar.gz"])
                    .pick_file();
                *schema_import_browse().lock().unwrap() = Some(picked);
            }
        });
    }

    pub fn paste_import_clipboard(&mut self) {
        #[cfg(windows)]
        if let Some(path) = clipboard_file_path() {
            self.schema_import.set_path(path.display().to_string());
            return;
        }
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) if !text.trim().is_empty() => self.schema_import.set_clipboard_text(text),
            _ => self.schema_import.message = Some("剪贴板里没有文件路径或 URL".into()),
        }
    }

    pub fn delete_market_package(&mut self, schema_id: &str) {        // 与 delete_market_model 同型守卫：下载/安装中删包会让下载线程
        // 往已删目录回写残缺包、或让安装线程从被删目录读包。
        if self.market_schema.downloading.as_deref() == Some(schema_id)
            || self.market_schema.installing.as_deref() == Some(schema_id)
        {
            return;
        }
        if self.market_schema.installed_ids.iter().any(|id| id == schema_id) {
            self.market_schema.install_message =
                Some("该方案包已安装，请先卸载再删除下载包".to_string());
            self.market_schema.install_message_since = Some(std::time::Instant::now());
            return;
        }
        let sid = schema_id.to_string();
        std::thread::spawn(move || {
            let pkg_dir = market_dir().join(&sid);
            let _ = std::fs::remove_dir_all(&pkg_dir);
            let task = MarketTaskResult::DeleteDone(sid);
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn uninstall_market_schema(&mut self, schema_id: &str) {
        if self.market_schema.installing.is_some() || self.market_schema.downloading.is_some() {
            return;
        }

        self.market_schema.installing = Some(schema_id.to_string());
        self.market_schema.install_message = None;

        let sid = schema_id.to_string();
        std::thread::spawn(move || {
            let result = do_uninstall(&sid, true);
            let task = match result {
                Ok(()) => {
                    notify_deploy_toast("方案已卸载", &format!("{sid} 已移除并重新部署"));
                    MarketTaskResult::UninstallDone
                }
                Err(e) => {
                    notify_deploy_toast("方案卸载失败", &format!("{sid}：{e}"));
                    MarketTaskResult::Error(e.to_string())
                }
            };
            *market_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn download_market_model(&mut self, model_id: &str) {
        if self.market_model.downloading.is_some() {
            return;
        }

        let model = match self.market_model.models.iter().find(|m| m.id == model_id) {
            Some(m) => m.clone(),
            None => return,
        };

        self.market_model.downloading = Some(model_id.to_string());
        self.market_model.download_progress = None;
        self.market_model.install_message = None;

        std::thread::spawn(move || {
            let result = do_download_model(&model);
            let task = match result {
                Ok(()) => ModelTaskResult::DownloadDone(model.id.clone()),
                Err(e) => ModelTaskResult::Error(e.to_string()),
            };
            *model_task_result().lock().unwrap() = Some(task);
        });
    }

    pub fn delete_market_model(&mut self, model_id: &str) {
        if self.market_model.downloading.as_deref() == Some(model_id) {
            return;
        }
        let mid = model_id.to_string();
        std::thread::spawn(move || {
            let model_dir = models_dir().join(&mid);
            let _ = std::fs::remove_dir_all(&model_dir);
            let task = ModelTaskResult::DeleteDone(mid);
            *model_task_result().lock().unwrap() = Some(task);
        });
    }

    // ---- 私有辅助 ----

    /// 已安装的方案包 id（注册表为准；内置方案包不计入市场包列表）。
    fn get_installed_package_ids(&self) -> Vec<String> {
        schema_manifest()
            .map(|m| {
                m.installed_packages()
                    .into_iter()
                    .filter(|pkg| pkg != BUILTIN_PACKAGE_ID)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn get_cached_schema_ids(&self) -> Vec<String> {
        downloaded_schema_ids()
    }

    fn get_cached_model_ids(&self) -> Vec<String> {
        scan_dir_ids(&models_dir())
    }
}

/// 扫描 market 下**真正含压缩包**的已下载方案包。
///
/// 只按目录存在判断会把「下载失败/取消留下的空目录」也算成已下载：商店显示
/// 「已下载」但已下载页因为过滤了空目录而完全没有卡片，用户也无处删除。
pub(crate) fn downloaded_schema_ids() -> Vec<String> {
    scan_downloaded_schema_ids(&market_dir())
}

fn scan_downloaded_schema_ids(dir: &std::path::Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if name.starts_with('.') || name == BUILTIN_PACKAGE_ID || !entry.path().is_dir() {
                continue;
            }
            let has_archive = std::fs::read_dir(entry.path())
                .map(|entries| {
                    entries.flatten().any(|file| {
                        let lower = file.file_name().to_string_lossy().to_ascii_lowercase();
                        file.path().is_file()
                            && (lower.ends_with(".zip") || lower.ends_with(".tar.gz"))
                    })
                })
                .unwrap_or(false);
            if has_archive {
                ids.push(name);
            }
        }
    }
    ids
}

/// 扫描目录下的子目录名（跳过隐藏项），作为已下载/已缓存列表。
fn scan_dir_ids(dir: &std::path::Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if entry.path().is_dir() {
                ids.push(name);
            }
        }
    }
    ids
}

/// 扫 `build/` 目录得到「已部署方案 id」：有 `<id>.schema.yaml` 编译产物才算已部署。
///
/// 判据与引擎一致（`RimeEngine::schema_deployed`），也是服务端拒绝切换的依据，
/// 所以设置页用它区分「能用」与「只是文件在、未启用」。
fn deployed_schema_ids_in(build_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(build_dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".schema.yaml"))
                .map(String::from)
        })
        .collect();
    ids.sort();
    ids
}

fn deployed_schema_ids() -> Vec<String> {
    let (_, user_data_dir) = get_data_dirs();
    deployed_schema_ids_in(&user_data_dir.join("build"))
}

fn trim_import_input(text: &str) -> String {
    text.trim().trim_matches('"').trim().to_string()
}

fn archive_name_ok(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz")
}

fn sanitize_package_id(raw: &str) -> Option<String> {
    let raw = trim_import_input(raw);
    let lower = raw.to_ascii_lowercase();
    let stem: &str = if lower.ends_with(".tar.gz") && raw.len() > 7 {
        &raw[..raw.len() - 7]
    } else if lower.ends_with(".zip") && raw.len() > 4 {
        &raw[..raw.len() - 4]
    } else {
        raw.as_str()
    };
    let id: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = id.trim_matches(['.', '-', ' ']).to_string();
    if id.is_empty() || id == "." || id == ".." {
        None
    } else {
        Some(id)
    }
}

/// 自动推导一个面向用户的包名：`rime-ice` → `Rime Ice`，中文保持原样。
fn humanize_default_id(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn package_id_from_path(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_string_lossy();
    if !archive_name_ok(&name) {
        return None;
    }
    sanitize_package_id(&name).map(|id| humanize_default_id(&id))
}

fn filename_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("");
    if name.is_empty() || !archive_name_ok(name) {
        return None;
    }
    Some(name.to_string())
}

fn package_id_from_url(url: &str) -> Option<String> {
    filename_from_url(url).and_then(|name| {
        sanitize_package_id(&name).map(|id| humanize_default_id(&id))
    })
}

/// 把来源复制/下载到 `market/<id>/`；返回包 id，供正常安装流程使用。
fn prepare_import(path: &str, url: &str, id: &str) -> Result<String, String> {
    let dest_dir = market_dir().join(id);
    if dest_dir.exists() {
        return Err(format!("方案名称「{id}」已存在"));
    }
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建缓存目录失败: {e}"))?;

    let result = (|| -> Result<(), String> {
        let filename = if !path.is_empty() {
            let source = std::path::Path::new(path);
            if !source.is_file() {
                return Err(format!("本地文件不存在: {path}"));
            }
            let name = source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or_else(|| "无法识别文件名".to_string())?;
            if !archive_name_ok(&name) {
                return Err("只支持 .zip / .tar.gz 方案包".to_string());
            }
            let size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                *schema_import_progress().lock().unwrap() = Some((0, size, 0));
            }
            std::fs::copy(source, dest_dir.join(&name))
                .map_err(|e| format!("复制方案包失败: {e}"))?;
            if size > 0 {
                *schema_import_progress().lock().unwrap() = Some((size, size, 0));
            }
            name
        } else {
            let name = filename_from_url(url)
                .ok_or_else(|| "URL 必须以 .zip / .tar.gz 结尾".to_string())?;
            download_file_with_stats(url, &dest_dir.join(&name), None, |done, total, bps| {
                *schema_import_progress().lock().unwrap() = Some((done, total, bps));
            })
            .map_err(|e| format!("下载失败: {e}"))?;
            name
        };
        if !archive_name_ok(&filename) {
            return Err("只支持 .zip / .tar.gz 方案包".to_string());
        }
        Ok(())
    })();

    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dest_dir);
        return Err(e);
    }
    Ok(id.to_string())
}

#[cfg(windows)]
fn clipboard_file_path() -> Option<std::path::PathBuf> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    const CF_HDROP: u32 = 15;
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let handle = match GetClipboardData(CF_HDROP) {
            Ok(handle) => handle,
            Err(_) => {
                let _ = CloseClipboard();
                return None;
            }
        };
        let hdrop = HDROP(handle.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        if count == 0 {
            let _ = CloseClipboard();
            return None;
        }
        let mut buf = [0u16; 1024];
        let len = DragQueryFileW(hdrop, 0, Some(&mut buf));
        let _ = CloseClipboard();
        if len == 0 {
            return None;
        }
        Some(std::path::PathBuf::from(String::from_utf16_lossy(
            &buf[..(len as usize).min(buf.len())],
        )))
    }
}

/// 重扫方案列表（纯读盘，可在后台线程跑）。
fn load_schema_snapshot() -> SchemaSnapshot {
    let mut schemas = Vec::new();
    let mut packages = Vec::new();
    let mut builtin_restorable = false;

    if let Ok(manager) = SchemaManager::new() {
        schemas = manager.get_schema_list();
        packages = vec![BUILTIN_PACKAGE_ID.to_string(); schemas.len()];
        if let Ok(manifest) = schema_manifest() {
            let mut registry = manifest.load_registry();
            // 注册表里还没有内置方案包（首次运行 / 从未安装过市场包）：
            // 把 rime 目录里的无主方案文件登记为内置方案包。
            if !registry.contains_key(BUILTIN_PACKAGE_ID) {
                let _ = manifest.refresh_builtin_package();
                registry = manifest.load_registry();
            }
            for (i, schema) in schemas.iter().enumerate() {
                let rel = format!("{}.schema.yaml", schema.schema_id);
                if let Some(pkg) = SchemaManifest::package_of(&registry, &rel) {
                    packages[i] = pkg;
                }
            }
            // 内置方案可一键还原的判定：market/builtin/ 备份存在，且当前
            // builtin 条目里没有任何方案文件——包括被卸载（注册表无条目）和
            // 只剩无主共享词典的残缺条目。
            let builtin_has_schema = registry
                .get(BUILTIN_PACKAGE_ID)
                .map(|entry| entry.files.iter().any(|f| f.ends_with(".schema.yaml")))
                .unwrap_or(false);
            builtin_restorable =
                !builtin_has_schema && !manifest.builtin_backup_files().is_empty();
        }
    }

    SchemaSnapshot {
        schemas,
        packages,
        builtin_restorable,
        deployed: deployed_schema_ids(),
    }
}

/// 安装/卸载/还原完成后：后台重扫，UI 下一次 `poll_schema_reload` 一次性替换。
fn start_schema_reload_async() {
    if SCHEMA_RELOADING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let snapshot = load_schema_snapshot();
        match schema_snapshot().lock() {
            Ok(mut slot) => *slot = Some(snapshot),
            Err(poisoned) => *poisoned.into_inner() = Some(snapshot),
        }
        SCHEMA_RELOADING.store(false, Ordering::Release);
    });
}

/// 模型下载目录：~/.config/xime/models/<id>/。
fn models_dir() -> std::path::PathBuf {
    let (_, user_data_dir) = get_data_dirs();
    user_data_dir
        .parent()
        .map(|p| p.join("models"))
        .unwrap_or_else(|| {
            let base = std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            base.join(xime_config::app_metadata().config_dir_name)
                .join("models")
        })
}

/// 插件安装目录根：~/.config/xime/plugins/。
fn plugins_dir() -> std::path::PathBuf {
    let (_, user_data_dir) = get_data_dirs();
    user_data_dir
        .parent()
        .map(|p| p.join("plugins"))
        .unwrap_or_else(|| {
            let base = std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            base.join(xime_config::app_metadata().config_dir_name)
                .join("plugins")
        })
}

/// 构建插件管理器（注册表/配置都在 plugins 目录下）。
fn plugin_manager() -> xime_plugin::PluginManager {
    xime_plugin::PluginManager::new(plugins_dir())
}

#[cfg(feature = "smart-suggestion-page")]
#[derive(Clone, Default)]
pub struct SmartSuggestionState {
    pub enabled: bool,
    pub suggestion_count: i32,
    pub record_user_frequency: bool,
    pub auto_adjust_frequency: bool,
    pub learning_threshold: i32,
}

#[cfg(feature = "pair-page")]
#[derive(Clone, Default)]
pub struct PairState {}

#[cfg(feature = "clipboard-page")]
pub struct ClipboardState {
    /// 同步服务器配置文件（~/.config/xime/xime-sync.toml）。
    pub config_path: std::path::PathBuf,
    /// 监听地址（server.addr）。
    pub server_addr: String,
    /// 认证用户名（auth.username）。
    pub username: String,
    /// 认证密码（auth.password，写入配置文件，权限 0600）。
    pub password: String,
    /// 数据目录（server.data_dir）。
    pub data_dir: String,
    /// 服务器子进程是否运行。
    pub running: bool,
    /// 最近一次操作的状态消息。
    pub status_message: Option<String>,
    /// 服务器子进程句柄（仅启动后持有）。
    child: Option<std::process::Child>,
}

/// 设置程序管理的 server 配置片段（字段名与 xime-sync-server 配置对齐，
/// 其余字段由 server 用内嵌默认值补全）。
#[cfg(feature = "clipboard-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SyncConfigFile {
    #[serde(default)]
    server: SyncServerSection,
    #[serde(default)]
    auth: SyncAuthSection,
}

#[cfg(feature = "clipboard-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SyncServerSection {
    addr: Option<String>,
    data_dir: Option<String>,
}

#[cfg(feature = "clipboard-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SyncAuthSection {
    username: Option<String>,
    password: Option<String>,
}

#[cfg(feature = "clipboard-page")]
impl Default for ClipboardState {
    fn default() -> Self {
        Self::load()
    }
}

#[cfg(feature = "clipboard-page")]
impl Clone for ClipboardState {
    fn clone(&self) -> Self {
        Self {
            config_path: self.config_path.clone(),
            server_addr: self.server_addr.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            data_dir: self.data_dir.clone(),
            running: self.running,
            status_message: self.status_message.clone(),
            child: None,
        }
    }
}

#[cfg(feature = "clipboard-page")]
impl ClipboardState {
    pub fn load() -> Self {
        let config_path = sync_config_path();
        let mut st = Self {
            config_path,
            server_addr: "0.0.0.0:8443".to_string(),
            username: "xime".to_string(),
            password: String::new(),
            data_dir: sync_data_dir().to_string_lossy().into_owned(),
            running: false,
            status_message: None,
            child: None,
        };
        st.read_config();
        st
    }

    /// 从配置文件读取已有设置（缺失走默认值）。
    fn read_config(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.config_path) else {
            return;
        };
        let Ok(cfg) = toml::from_str::<SyncConfigFile>(&content) else {
            return;
        };
        if let Some(addr) = cfg.server.addr {
            self.server_addr = addr;
        }
        if let Some(dir) = cfg.server.data_dir {
            self.data_dir = dir;
        }
        if let Some(u) = cfg.auth.username {
            self.username = u;
        }
        if let Some(p) = cfg.auth.password {
            self.password = p;
        }
    }

    /// 保存配置到配置文件（0600 权限，密码明文仅本地可读）。
    fn write_config(&self) -> Result<(), String> {
        let cfg = SyncConfigFile {
            server: SyncServerSection {
                addr: Some(self.server_addr.clone()),
                data_dir: Some(self.data_dir.clone()),
            },
            auth: SyncAuthSection {
                username: Some(self.username.clone()),
                password: Some(self.password.clone()),
            },
        };
        let content = toml::to_string(&cfg).map_err(|e| e.to_string())?;
        let parent = self.config_path.parent().ok_or("配置目录无效")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // 含明文密码：必须 0600 一步创建——先 0644 写入再补 chmod 存在同机
        // 其他用户可读的窗口。
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.config_path)
                .map_err(|e| e.to_string())?;
            f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.config_path, content).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// 启动 xime-sync-server 子进程（派生独立进程，不随设置窗口关闭）。
    ///
    /// 密码为空时自动生成随机密码并持久化到配置（零配置启动，同 ximed 行为）。
    pub fn spawn_server(&mut self) -> Result<(), String> {
        if self.running {
            return Ok(());
        }
        if self.password.is_empty() {
            self.password = random_password();
            self.status_message = Some("已生成随机密码，客户端请使用设置页显示的密码".to_string());
        }
        self.write_config()?;
        let bin = std::env::var("XIME_SYNC_SERVER_BIN").unwrap_or_else(|_| {
            // 优先级：环境变量 > 设置程序同级目录（打包进 bundle）> ~/.local/bin > PATH
            let mut sibling = None;
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let candidate = dir.join("xime-sync-server");
                    if candidate.exists() {
                        sibling = Some(candidate.to_string_lossy().into_owned());
                    }
                }
            }
            sibling.unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_default();
                let candidate = std::path::PathBuf::from(&home).join(".local/bin/xime-sync-server");
                if candidate.exists() {
                    candidate.to_string_lossy().into_owned()
                } else {
                    "xime-sync-server".to_string()
                }
            })
        });
        let child = std::process::Command::new(&bin)
            .arg("--config")
            .arg(&self.config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动同步服务器失败: {e}"))?;
        let pid = child.id();
        self.running = true;
        self.child = Some(child);
        self.status_message = Some(format!("服务器已启动 (PID {pid})"));
        Ok(())
    }

    /// 停止服务器子进程。
    pub fn stop_server(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.running = false;
            self.status_message = Some("服务器已停止".to_string());
        }
    }

    /// 轮询子进程状态（外部崩溃/被手动结束后更新 UI）。
    pub fn poll(&mut self) {
        if let Some(child) = &mut self.child {
            if let Ok(Some(status)) = child.try_wait() {
                self.running = false;
                self.child = None;
                self.status_message = Some(format!("服务器已退出 ({status})"));
            }
        }
    }
}

/// 共享 blocking 客户端（reqwest blocking 每个实例内建 runtime 线程，必须复用）。
pub(crate) fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            // `timeout(0)` **不是**"不限"：reqwest 会把它当成零时长 → 请求立刻 TimedOut。
            // 商店/模型/插件三个索引与下载都走这个客户端，之前全部因此失败
            // （表现就是「网络错误」，而浏览器打开同一个 URL 一切正常）。
            // `None` 才是真正的不限总时长（连接阶段仍受上面的 connect_timeout 约束）。
            .timeout(None::<std::time::Duration>)
            .build()
            .expect("failed to build http client")
    })
}

#[cfg(any(feature = "clipboard-page", feature = "backup-page"))]
fn config_base_dir() -> std::path::PathBuf {
    let (_, user_data_dir) = get_data_dirs();
    user_data_dir
        .parent()
        .unwrap_or(&user_data_dir)
        .to_path_buf()
}

#[cfg(feature = "clipboard-page")]
fn sync_config_path() -> std::path::PathBuf {
    config_base_dir().join("xime-sync.toml")
}

#[cfg(feature = "clipboard-page")]
fn sync_data_dir() -> std::path::PathBuf {
    config_base_dir().join("sync-data")
}

// ---- 剪贴板同步插件（clipboard-page + 与 IME 共享配置） ----------------------

/// 剪贴板同步插件发现结果。
#[cfg(feature = "clipboard-page")]
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardSyncPluginInfo {
    pub id: String,
    pub name: String,
    pub dir: std::path::PathBuf,
    /// 注册表中的启用状态（无记录默认 true，与 daemon 扫描一致）。
    pub enabled: bool,
}

/// 插件同步配置（clipboard_sync.toml，与 IME 共享）。
#[cfg(feature = "clipboard-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct ClipboardSyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub plugin_id: String,
}

#[cfg(feature = "clipboard-page")]
pub fn clipboard_sync_config_path() -> std::path::PathBuf {
    config_base_dir().join("clipboard_sync.toml")
}

#[cfg(feature = "clipboard-page")]
pub fn read_clipboard_sync_config() -> ClipboardSyncConfig {
    let Ok(content) = std::fs::read_to_string(clipboard_sync_config_path()) else {
        return Default::default();
    };
    toml::from_str(&content).unwrap_or_default()
}

#[cfg(feature = "clipboard-page")]
pub fn write_clipboard_sync_config(cfg: &ClipboardSyncConfig) {
    let path = clipboard_sync_config_path();
    if let Ok(content) = toml::to_string(cfg) {
        let _ = std::fs::write(path, content);
    }
}

/// 扫描全部已安装的 clipboard_sync 类型插件（含未启用，对齐 Android
/// `getAllInstalledPlugins` 按分类过滤：下拉选择即激活，不应把停用的藏起来）。
#[cfg(feature = "clipboard-page")]
pub fn scan_clipboard_sync_plugins() -> Vec<ClipboardSyncPluginInfo> {
    let root = config_base_dir().join("plugins");
    let manager = xime_plugin::PluginManager::new(&root);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let Ok(manifest) = xime_plugin::PluginManifest::from_dir(&dir) else {
            continue;
        };
        if manifest.plugin_type() != xime_plugin::PluginType::ClipboardSync {
            continue;
        }
        let enabled = manager.get(&manifest.id).map(|r| r.enabled).unwrap_or(true);
        out.push(ClipboardSyncPluginInfo {
            id: manifest.id,
            name: manifest.name,
            dir,
            enabled,
        });
    }
    out
}

// ------------------------------------------------------------------
// 剪贴板历史（SQLite：xime_config::clipboard_store，与 server 共享 clipboard.db）
// ------------------------------------------------------------------

/// 列表分页：每页条数（2 列 × 4 行卡片）。
#[cfg(feature = "clipboard-page")]
pub const CLIPBOARD_PAGE_SIZE: usize = 8;

/// 剪贴板历史页面状态：SQLite 读取展示 + 清空。
#[cfg(feature = "clipboard-page")]
#[derive(Clone)]
pub struct ClipboardHistoryState {
    /// 最新在前。
    pub items: Vec<xime_config::clipboard_store::ClipboardHistoryItem>,
    /// 当前选中的历史条目 id（点击卡片选中，再点取消）。
    pub selected: Option<i64>,
    /// 当前页（0 起）。
    pub page: usize,
}

#[cfg(feature = "clipboard-page")]
impl ClipboardHistoryState {
    pub fn load() -> Self {
        let db = xime_config::clipboard_store::default_db_path();
        xime_config::clipboard_store::migrate_legacy(
            &db,
            &db.with_file_name("clipboard_history.json"),
            &db.with_file_name("quick_send.yaml"),
        );
        Self {
            items: xime_config::clipboard_store::list_history(&db, 50).unwrap_or_default(),
            selected: None,
            page: 0,
        }
    }

    pub fn reload(&mut self) {
        self.items = xime_config::clipboard_store::list_history(
            &xime_config::clipboard_store::default_db_path(),
            50,
        )
        .unwrap_or_default();
        self.clamp_page();
    }

    /// 总页数（至少 1）。
    pub fn total_pages(&self) -> usize {
        self.items.len().div_ceil(CLIPBOARD_PAGE_SIZE).max(1)
    }

    /// 列表变化后把当前页夹回有效范围。
    pub fn clamp_page(&mut self) {
        self.page = self.page.min(self.total_pages() - 1);
    }

    pub fn prev_page(&mut self) {
        self.page = self.page.saturating_sub(1);
    }

    pub fn next_page(&mut self) {
        if self.page + 1 < self.total_pages() {
            self.page += 1;
        }
    }

    /// 点击卡片：选中 / 再点取消。
    pub fn select(&mut self, id: i64) {
        self.selected = if self.selected == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// 删除单条历史。
    pub fn remove(&mut self, id: i64) {
        let _ = xime_config::clipboard_store::remove_history_item(
            &xime_config::clipboard_store::default_db_path(),
            id,
        );
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.reload();
    }

    /// 清空（保留快捷发送条目）。
    pub fn clear(&mut self) {
        let _ = xime_config::clipboard_store::clear_history(
            &xime_config::clipboard_store::default_db_path(),
        );
        self.items.clear();
        self.selected = None;
        self.page = 0;
    }
}

// ------------------------------------------------------------------
// 快捷发送（clipboard_entries 表 isQuickSend=1 子集，对齐 Android QuickSendItem）
// ------------------------------------------------------------------

/// 快捷发送页面状态：SQLite 列表 + 新增弹窗。
#[cfg(feature = "clipboard-page")]
#[derive(Clone)]
pub struct QuickSendState {
    pub items: Vec<xime_config::clipboard_store::QuickSendItem>,
    /// 当前选中的快捷发送条目 id（点击卡片选中，再点取消）。
    pub selected: Option<i64>,
    /// 当前页（0 起）。
    pub page: usize,
    /// 新增弹窗是否打开。
    pub dialog_open: bool,
    /// 新增弹窗草稿（内容 / 触发编码输入框）。
    pub draft_content: String,
    pub draft_code: String,
}

#[cfg(feature = "clipboard-page")]
impl QuickSendState {
    pub fn load() -> Self {
        let db = xime_config::clipboard_store::default_db_path();
        xime_config::clipboard_store::migrate_legacy(
            &db,
            &db.with_file_name("clipboard_history.json"),
            &db.with_file_name("quick_send.yaml"),
        );
        Self {
            items: xime_config::clipboard_store::list_quick_send(&db).unwrap_or_default(),
            selected: None,
            page: 0,
            dialog_open: false,
            draft_content: String::new(),
            draft_code: String::new(),
        }
    }

    pub fn reload(&mut self) {
        self.items = xime_config::clipboard_store::list_quick_send(
            &xime_config::clipboard_store::default_db_path(),
        )
        .unwrap_or_default();
        self.clamp_page();
    }

    /// 总页数（至少 1）。
    pub fn total_pages(&self) -> usize {
        self.items.len().div_ceil(CLIPBOARD_PAGE_SIZE).max(1)
    }

    /// 列表变化后把当前页夹回有效范围。
    pub fn clamp_page(&mut self) {
        self.page = self.page.min(self.total_pages() - 1);
    }

    pub fn prev_page(&mut self) {
        self.page = self.page.saturating_sub(1);
    }

    pub fn next_page(&mut self) {
        if self.page + 1 < self.total_pages() {
            self.page += 1;
        }
    }

    /// 点击卡片：选中 / 再点取消。
    pub fn select(&mut self, id: i64) {
        self.selected = if self.selected == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    pub fn set_draft_code(&mut self, v: String) {
        self.draft_code = v;
    }

    pub fn set_draft_content(&mut self, v: String) {
        self.draft_content = v;
    }

    /// 打开新增弹窗（清空上次草稿）。
    pub fn open_dialog(&mut self) {
        self.dialog_open = true;
        self.draft_content.clear();
        self.draft_code.clear();
    }

    pub fn cancel_dialog(&mut self) {
        self.dialog_open = false;
        self.draft_content.clear();
        self.draft_code.clear();
    }

    /// 确认添加（内容为空时忽略并保持弹窗打开）；成功后关闭弹窗并刷新列表。
    pub fn add_draft(&mut self) -> bool {
        let content = self.draft_content.trim().to_string();
        if content.is_empty() {
            return false;
        }
        let code = self.draft_code.trim().to_string();
        let db = xime_config::clipboard_store::default_db_path();
        if xime_config::clipboard_store::add_quick_send(&db, &content, &code).is_err() {
            return false;
        }
        self.cancel_dialog();
        self.reload();
        true
    }

    pub fn remove(&mut self, id: i64) {
        let db = xime_config::clipboard_store::default_db_path();
        let _ = xime_config::clipboard_store::remove_quick_send(&db, id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.reload();
    }

    /// 从剪贴板历史文本直接添加为快捷发送（无触发编码）。
    pub fn add_from_text(&mut self, text: &str) -> bool {
        let content = text.trim();
        if content.is_empty() {
            return false;
        }
        let db = xime_config::clipboard_store::default_db_path();
        if xime_config::clipboard_store::add_quick_send(&db, content, "").is_err() {
            return false;
        }
        self.reload();
        true
    }
}

// ------------------------------------------------------------------
// rime 用户资料同步（sync_user_data：用户词典快照导出/合并，多端互通基础）
// ------------------------------------------------------------------

/// rime 用户资料同步状态：本机标识 + sync 快照目录概况（对齐 weasel
/// 「用户资料同步」语义；快照目录可整体上云实现多端词库合并）。
#[cfg(feature = "backup-page")]
#[derive(Clone)]
pub struct RimeSyncState {
    /// installation.yaml 的 installation_id（各端快照目录的隔离键）。
    pub installation_id: String,
    /// 同步快照目录（<rime>/sync）。
    pub sync_dir: std::path::PathBuf,
    /// sync 目录下的设备 installation 列表（含本机；从未同步为空）。
    pub devices: Vec<String>,
    /// 上次同步（sync 目录修改时间的相对描述；从未同步为 None）。
    pub last_sync: Option<String>,
    /// 同步进行中（后台线程持有；期间「立即同步」按钮置灰防重入）。
    pub syncing: bool,
}

/// 同步结果信箱（后台线程 → UI 轮询；锁毒化视为空）。
#[cfg(feature = "backup-page")]
static RIME_SYNC_OUTCOME: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

#[cfg(feature = "backup-page")]
impl RimeSyncState {
    pub fn load() -> Self {
        let (_, rime_dir) = get_data_dirs();
        let sync_dir = rime_dir.join("sync");
        // installation.yaml 形如 `installation_id: "uuid"`（引号可有可无）。
        let installation_id = std::fs::read_to_string(rime_dir.join("installation.yaml"))
            .ok()
            .and_then(|content| {
                content.lines().find_map(|line| {
                    let rest = line.trim().strip_prefix("installation_id:")?;
                    let v = rest.trim().trim_matches('"').trim_matches('\'').to_string();
                    (!v.is_empty()).then_some(v)
                })
            })
            .unwrap_or_default();
        let mut devices: Vec<String> = std::fs::read_dir(&sync_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        devices.sort();
        let last_sync = std::fs::metadata(&sync_dir)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(relative_time_ago);
        Self {
            installation_id,
            sync_dir,
            devices,
            last_sync,
            syncing: false,
        }
    }

    pub fn reload(&mut self) {
        let syncing = self.syncing;
        *self = Self::load();
        self.syncing = syncing;
    }

    /// 发起用户资料同步（后台线程执行；同步是 DBus 往返 + librime 快照导出，
    /// 词典大时数秒——绝不能在 UI 线程同步等待，否则整个窗口冻结）。
    pub fn start_sync(&mut self) -> bool {
        if self.syncing {
            return false;
        }
        self.syncing = true;
        std::thread::spawn(|| {
            let ok = notify_sync_user_data();
            *RIME_SYNC_OUTCOME
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ok);
        });
        true
    }

    /// BackgroundPoll 节拍：回收同步结果。返回结果文案（有结果时），
    /// 调用方经 `show_message` 呈现。
    pub fn poll_sync(&mut self) -> Option<Result<String, String>> {
        let outcome = RIME_SYNC_OUTCOME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let ok = outcome?;
        self.syncing = false;
        if ok {
            self.reload();
            Some(Ok("用户资料同步完成".to_string()))
        } else {
            Some(Err("用户资料同步失败（输入法服务未运行？）".to_string()))
        }
    }
}

/// 距今时长 → 人话（刚刚 / N 分钟前 / N 小时前 / N 天前）。
#[cfg(feature = "backup-page")]
fn relative_time_ago(elapsed: std::time::Duration) -> String {
    let mins = elapsed.as_secs() / 60;
    if mins < 1 {
        "刚刚".to_string()
    } else if mins < 60 {
        format!("{mins} 分钟前")
    } else if mins < 60 * 24 {
        format!("{} 小时前", mins / 60)
    } else {
        format!("{} 天前", mins / (60 * 24))
    }
}

// ------------------------------------------------------------------
// 语音转文本（Windows WinRT 听写；speech.rs worker 的 UI 侧镜像）
// ------------------------------------------------------------------

/// 语音转文本页面状态：worker 句柄 + 结果槽镜像（250ms 轮询刷新）。
#[cfg(all(feature = "voice-page", windows))]
#[derive(Clone, Default)]
pub struct SpeechState {
    /// 语音 worker 句柄（首次开始时创建；None = 从未启动）。
    handle: Option<crate::speech::VoiceHandle>,
    /// 正在听写（Listening）。
    pub listening: bool,
    /// 引擎处理中（启动/约束编译）。
    pub processing: bool,
    /// 累计识别文本（镜像 sink.text）。
    pub text: String,
    /// 最近一次错误。
    pub error: Option<String>,
}

#[cfg(all(feature = "voice-page", windows))]
impl SpeechState {
    /// BackgroundPoll 节拍：把 worker 结果槽镜像到 UI 状态。
    pub fn poll(&mut self) {
        let Some(h) = &self.handle else {
            return;
        };
        let sink = h.sink.lock().unwrap_or_else(|e| e.into_inner());
        self.text = sink.text.clone();
        self.listening = sink.state == crate::speech::RecognitionState::Listening;
        self.processing = sink.state == crate::speech::RecognitionState::Processing;
        if sink.state == crate::speech::RecognitionState::Error {
            self.error = sink.error.clone();
        }
    }

    /// 开始/停止听写（worker 首次使用时创建）。
    pub fn toggle(&mut self) {
        let h = self
            .handle
            .get_or_insert_with(crate::speech::VoiceHandle::spawn);
        if self.listening || self.processing {
            h.stop();
        } else {
            self.error = None;
            h.start();
        }
    }

    /// 清空累计文本（连同 worker 槽）。
    pub fn clear(&mut self) {
        if let Some(h) = &self.handle {
            let mut sink = h.sink.lock().unwrap_or_else(|e| e.into_inner());
            sink.text.clear();
        }
        self.text.clear();
    }

    /// 复制识别文本到系统剪贴板。
    pub fn copy_text(&self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        match arboard::Clipboard::new() {
            Ok(mut clip) => clip.set_text(self.text.clone()).is_ok(),
            Err(_) => false,
        }
    }
}

// ------------------------------------------------------------------
// 词典管理（用户词典列表 + 备份/恢复/导出/导入，对齐 weasel DictManagementDialog）
// ------------------------------------------------------------------

/// 词典后台任务结果（后台线程 → UI 轮询）。
#[cfg(any(windows, feature = "dict-page"))]
enum DictTaskResult {
    List(DictListResult),
    Message(String),
    /// 词条读取成功（附带总数 + 已过滤词条；带词典名——在途期间切换
    /// 词典下拉的旧结果按词典名丢弃）。
    Entries {
        dict: String,
        result: DictEntriesResult,
    },
    /// 词条读取失败（原因；带词典名，同上）。
    EntriesFailed {
        dict: String,
        reason: String,
    },
}

#[cfg(any(windows, feature = "dict-page"))]
static DICT_TASK_OUTCOME: std::sync::Mutex<Option<DictTaskResult>> = std::sync::Mutex::new(None);

/// 词条列表每页条数。
///
/// 页面外层已经是滚动容器（`pages::scrollable_content`），整页铺几百行会让
/// 每帧构建变慢，所以这里沿用剪贴板页的分页做法。
#[cfg(any(windows, feature = "dict-page"))]
pub const DICT_ENTRIES_PAGE_SIZE: usize = 50;

/// 关键词输入的防抖时长：打字停下后才真正去扫一遍词库。
#[cfg(any(windows, feature = "dict-page"))]
const DICT_QUERY_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// 写词条 / 短语保存的后台结果信箱（与读取信箱分开：同一时间可能有
/// 一个在途读取 + 一个在途写入，共用一个槽会互相覆盖结果）。
#[cfg(any(windows, feature = "dict-page"))]
static DICT_WRITE_OUTCOME: std::sync::Mutex<Option<DictWriteResult>> = std::sync::Mutex::new(None);
/// 方案词表读取的结果信箱（输入方案页）。
#[cfg(any(windows, feature = "dict-page"))]
static SCHEMA_DICT_OUTCOME: std::sync::Mutex<Option<SchemaDictTaskResult>> =
    std::sync::Mutex::new(None);
/// 快捷短语读取/保存的结果信箱（词典页短语子视图）。
#[cfg(any(windows, feature = "dict-page"))]
static PHRASE_OUTCOME: std::sync::Mutex<Option<PhraseTaskResult>> = std::sync::Mutex::new(None);

/// 新增词条对话框的草稿。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct DictEntryDraft {
    /// 词。
    pub word: String,
    /// 编码。
    pub code: String,
    /// 频率（文本框；空串 = 1）。
    pub commits: String,
}

/// 用户词典「浏览词条」子视图状态。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct DictBrowseState {
    /// 正在浏览的词典名。
    pub dict: String,
    /// 搜索关键词（匹配词或编码，大小写不敏感）。
    pub query: String,
    /// 词库词条总数（未过滤）。
    pub total: i32,
    /// 命中条数（未截断；`entries` 被截断时用它提示用户）。
    pub matched: i32,
    /// 当前展示的词条（已按关键词过滤）。
    pub entries: Vec<DictEntryRow>,
    /// 当前页（0 起）。
    pub page: usize,
    /// 是否已经读过一次（区分"空词库"与"还没读"）。
    pub loaded: bool,
    /// 正在读取。
    pub loading: bool,
    /// 读取失败原因。
    pub error: Option<String>,
    /// 待发起的读取（关键词变化后等防抖到点）。
    pending: Option<std::time::Instant>,
    /// 正在写入（新增/删除在途；写按钮全部禁用，避免并发改词库）。
    pub writing: bool,
    /// 写入结果提示（成功后给用户看的一句）。
    pub notice: Option<String>,
    /// 待删除的词条（词, 编码)——两步确认。
    pub delete_confirm: Option<(String, String)>,
    /// 新增词条对话框（None = 关闭）。
    pub add_dialog: Option<DictEntryDraft>,
}

#[cfg(any(windows, feature = "dict-page"))]
impl DictBrowseState {
    /// 过滤后词条占多少页（0 表示没有词条）。
    pub fn page_count(&self) -> usize {
        self.entries.len().div_ceil(DICT_ENTRIES_PAGE_SIZE)
    }

    /// 当前页要显示的词条。
    pub fn page_entries(&self) -> &[DictEntryRow] {
        let start = self.page * DICT_ENTRIES_PAGE_SIZE;
        if start >= self.entries.len() {
            return &[];
        }
        let end = (start + DICT_ENTRIES_PAGE_SIZE).min(self.entries.len());
        &self.entries[start..end]
    }

    /// 命中数是否已被截断（提示用户补充关键词）。
    pub fn truncated(&self) -> bool {
        (self.entries.len() as i32) < self.matched
    }

    /// 浏览态的状态文案（加载中 / 失败 / 统计）。
    pub fn status_text(&self) -> String {
        if self.loading {
            return "正在读取词条…".to_string();
        }
        if let Some(error) = &self.error {
            return error.clone();
        }
        if !self.loaded {
            return "尚未读取".to_string();
        }
        let mut summary = if self.query.trim().is_empty() {
            format!("共 {} 条词条，显示前 {} 条", self.total, self.entries.len())
        } else {
            format!("共 {} 条词条，匹配 {} 条", self.total, self.matched)
        };
        if self.truncated() {
            summary.push_str(&format!(
                "（命中超过 {DICT_ENTRIES_MAX} 条，只列出前 {DICT_ENTRIES_MAX} 条，请补充关键词）"
            ));
        }
        summary
    }
}

/// 快捷短语编辑对话框的草稿（新增与编辑共用）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct PhraseDialogState {
    /// 编辑第几条（None = 新增）。
    pub editing: Option<usize>,
    /// 词。
    pub word: String,
    /// 编码。
    pub code: String,
    /// 权重（文本框；空串 = 省略该列）。
    pub weight: String,
}

/// 快捷短语子视图状态（词典页内：按方案编辑 `custom_phrase.txt`）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct CustomPhraseState {
    /// 正在编辑的方案 id。
    pub schema_id: String,
    /// 方案显示名（只展示）。
    pub schema_name: String,
    /// 短语表名（server 解析，回显用）。
    pub dict_name: String,
    /// 短语表文件名。
    pub file_name: String,
    /// 短语表文件是否已存在。
    pub file_exists: bool,
    /// 方案 custom.yaml 里是否已注入翻译器。
    pub patch_applied: bool,
    /// 本次会话里是否新注入过翻译器（提示"重新部署后生效"）。
    pub patch_added: bool,
    /// 短语列表（本地权威副本：每次改动整表保存到 server）。
    pub entries: Vec<CustomPhraseRow>,
    /// 是否已经读过一次。
    pub loaded: bool,
    /// 正在读取。
    pub loading: bool,
    /// 正在保存。
    pub saving: bool,
    /// 读取/保存失败原因。
    pub error: Option<String>,
    /// 操作结果提示。
    pub notice: Option<String>,
    /// 待删除的条目下标（两步确认）。
    pub delete_confirm: Option<usize>,
    /// 新增/编辑对话框（None = 关闭）。
    pub dialog: Option<PhraseDialogState>,
}

/// 方案词表浏览状态（输入方案页「方案词表」tab，只读）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone, Debug, Default)]
pub struct SchemaDictState {
    /// 正在浏览的方案 id（跟输入方案页选中行同步）。
    pub schema_id: String,
    /// 搜索关键词。
    pub query: String,
    /// 读入的词条总数。
    pub total: i32,
    /// 命中条数（未截断）。
    pub matched: i32,
    /// 当前展示的词条。
    pub entries: Vec<DictEntryRow>,
    /// 方案主码表名。
    pub dict_name: String,
    /// 实际读入的码表名。
    pub tables: Vec<String>,
    /// 声明了但文件不存在的码表名。
    pub missing: Vec<String>,
    /// 当前页（0 起）。
    pub page: usize,
    /// 是否已经读过一次。
    pub loaded: bool,
    /// 正在读取。
    pub loading: bool,
    /// 读取失败原因。
    pub error: Option<String>,
    /// 待发起的读取（关键词变化后等防抖到点）。
    pending: Option<std::time::Instant>,
}

#[cfg(any(windows, feature = "dict-page"))]
impl SchemaDictState {
    /// 过滤后词条占多少页（0 表示没有词条）。
    pub fn page_count(&self) -> usize {
        self.entries.len().div_ceil(DICT_ENTRIES_PAGE_SIZE)
    }

    /// 当前页要显示的词条。
    pub fn page_entries(&self) -> &[DictEntryRow] {
        let start = self.page * DICT_ENTRIES_PAGE_SIZE;
        if start >= self.entries.len() {
            return &[];
        }
        let end = (start + DICT_ENTRIES_PAGE_SIZE).min(self.entries.len());
        &self.entries[start..end]
    }

    /// 命中数是否已被截断。
    pub fn truncated(&self) -> bool {
        (self.entries.len() as i32) < self.matched
    }

    /// 状态文案（加载中 / 失败 / 统计）。
    pub fn status_text(&self) -> String {
        if self.loading {
            return "正在读取词表…".to_string();
        }
        if let Some(error) = &self.error {
            return error.clone();
        }
        if !self.loaded {
            return "尚未读取".to_string();
        }
        let mut summary = if self.query.trim().is_empty() {
            format!("共 {} 条词条，显示前 {} 条", self.total, self.entries.len())
        } else {
            format!("共 {} 条词条，匹配 {} 条", self.total, self.matched)
        };
        if self.truncated() {
            summary.push_str(&format!(
                "（命中超过 {DICT_ENTRIES_MAX} 条，只列出前 {DICT_ENTRIES_MAX} 条，请补充关键词）"
            ));
        }
        summary
    }
}

/// 方案词表后台任务结果（带 schema_id：在途期间换方案/换页时丢弃旧结果）。
#[cfg(any(windows, feature = "dict-page"))]
enum SchemaDictTaskResult {
    Entries {
        schema_id: String,
        result: SchemaEntriesResult,
    },
    Failed {
        schema_id: String,
        reason: String,
    },
}

/// 快捷短语后台任务结果（带 schema_id：在途期间换方案时丢弃旧结果）。
#[cfg(any(windows, feature = "dict-page"))]
enum PhraseTaskResult {
    Loaded {
        schema_id: String,
        result: PhraseListResult,
    },
    Saved {
        schema_id: String,
        result: PhraseSaveResult,
    },
    Failed {
        schema_id: String,
        reason: String,
    },
}

/// 词典管理页面状态。
///
/// 版式为「页内 Tab + 表格」（对齐剪贴板页 / 小狼毫词典管理对话框）：
/// Tab 0 用户词典（词典下拉 + 搜索 + 词条表格 + 整本操作），
/// Tab 1 快捷短语（方案下拉 + 短语表格 + 部署）。
#[cfg(any(windows, feature = "dict-page"))]
#[derive(Clone)]
pub struct DictManageState {
    /// 用户词典名列表。
    pub dicts: Vec<String>,
    /// 快照目录（同步目录）。
    pub sync_dir: String,
    /// 进行中的操作标识（refresh/backup/restore/export/import）。
    pub busy: Option<&'static str>,
    /// 最近一次操作的结果消息。
    pub message: Option<String>,
    /// 页内当前 Tab（0=用户词典 1=快捷短语）。
    pub tab: usize,
    /// 用户词典 Tab 的词条浏览状态（常驻；`dict` = 当前选中词典，
    /// 空 = 尚未选中——列表到达时自动选第一本）。
    pub browse: DictBrowseState,
    /// 快捷短语 Tab 的状态（None = 还没进过该 Tab；进入时自动为当前
    /// 方案载入）。
    pub phrase: Option<CustomPhraseState>,
}

#[cfg(any(windows, feature = "dict-page"))]
impl Default for DictManageState {
    /// 启动即拉一次词典列表：列表为空时页面上连词典下拉都是空的，
    /// 不该让用户先点一次「刷新」才能看到功能。
    fn default() -> Self {
        let mut state = Self {
            dicts: Vec::new(),
            sync_dir: String::new(),
            busy: None,
            message: None,
            tab: 0,
            browse: DictBrowseState::default(),
            phrase: None,
        };
        state.start_refresh();
        state
    }
}

#[cfg(any(windows, feature = "dict-page"))]
impl DictManageState {
    fn submit(message: DictTaskResult) {
        *DICT_TASK_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
    }

    /// 刷新用户词典列表。
    pub fn start_refresh(&mut self) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("refresh");
        std::thread::spawn(|| {
            let result = match notify_dict_list() {
                Some(r) => DictTaskResult::List(r),
                None => {
                    DictTaskResult::Message("获取词典列表失败（输入法服务未运行？）".to_string())
                }
            };
            Self::submit(result);
        });
    }

    /// 备份词典快照到同步目录。
    pub fn start_backup(&mut self, dict: String) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("backup");
        std::thread::spawn(move || {
            let ok = notify_dict_backup(&dict);
            let msg = if ok {
                format!("已备份 {dict} 快照到同步目录")
            } else {
                format!("备份 {dict} 失败（输入法服务未运行？）")
            };
            Self::submit(DictTaskResult::Message(msg));
        });
    }

    /// 从快照文件恢复。
    pub fn start_restore(&mut self, path: String) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("restore");
        std::thread::spawn(move || {
            let ok = notify_dict_restore(&path);
            let msg = if ok {
                "已从快照恢复用户词典".to_string()
            } else {
                "恢复用户词典失败（快照无效或服务未运行？）".to_string()
            };
            Self::submit(DictTaskResult::Message(msg));
        });
    }

    /// 导出词典为文本。
    pub fn start_export(&mut self, dict: String, path: String) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("export");
        std::thread::spawn(move || {
            let msg = match notify_dict_export(&dict, &path) {
                Some(count) => format!("已导出 {dict}（{count} 条）"),
                None => format!("导出 {dict} 失败（输入法服务未运行？）"),
            };
            Self::submit(DictTaskResult::Message(msg));
        });
    }

    /// 从文本导入词典（合并去重）。
    pub fn start_import(&mut self, dict: String, path: String) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("import");
        std::thread::spawn(move || {
            let msg = match notify_dict_import(&dict, &path) {
                Some(count) => format!("已导入 {dict}（{count} 条）"),
                None => format!("导入 {dict} 失败（文本格式无效或服务未运行？）"),
            };
            Self::submit(DictTaskResult::Message(msg));
        });
    }

    /// 下拉切换当前词典：清掉旧词典的浏览状态并立刻读新词典。
    ///
    /// 词典名回显在读取结果里（`DictTaskResult::Entries { dict, .. }`），
    /// 切换时在途的旧结果会被 `poll` 丢弃；`loading` 复位让新读取立即出发。
    pub fn browse_select(&mut self, dict: String) {
        if self.browse.dict == dict {
            return;
        }
        self.message = None;
        self.browse.dict = dict;
        self.browse.query.clear();
        self.browse.entries.clear();
        self.browse.total = 0;
        self.browse.matched = 0;
        self.browse.page = 0;
        self.browse.loaded = false;
        self.browse.loading = false;
        self.browse.writing = false;
        self.browse.error = None;
        self.browse.notice = None;
        self.browse.add_dialog = None;
        self.browse.delete_confirm = None;
        self.start_entries_fetch();
    }

    /// 关键词变化：只记下来，等防抖到点由 `poll` 发起读取。
    pub fn browse_set_query(&mut self, query: String) {
        if self.browse.query == query {
            return;
        }
        self.browse.query = query;
        self.browse.page = 0;
        self.browse.pending = Some(std::time::Instant::now());
    }

    /// 翻页（越界自动夹到范围内）。
    pub fn browse_page(&mut self, page: usize) {
        let pages = self.browse.page_count();
        self.browse.page = if pages == 0 { 0 } else { page.min(pages - 1) };
    }

    /// 发起一次词条读取（单飞：已有读取在途时只记 pending，等结果回来再补）。
    fn start_entries_fetch(&mut self) {
        if self.browse.dict.is_empty() {
            return;
        }
        if self.browse.loading {
            self.browse.pending = Some(std::time::Instant::now());
            return;
        }
        self.browse.loading = true;
        self.browse.pending = None;
        self.browse.error = None;
        let dict = self.browse.dict.clone();
        let query = self.browse.query.clone();
        std::thread::spawn(move || {
            let result = match notify_dict_entries(&dict, &query) {
                Some(r) => DictTaskResult::Entries {
                    dict: dict.clone(),
                    result: r,
                },
                None => DictTaskResult::EntriesFailed {
                    dict: dict.clone(),
                    reason: "读取词条失败（输入法服务未运行？）".to_string(),
                },
            };
            Self::submit(result);
        });
    }

    // ---- 词条写入（新增 / 删除标记，走 ImportDictEntry） ----

    /// 打开新增词条对话框。
    pub fn browse_add_open(&mut self) {
        self.browse.notice = None;
        self.browse.error = None;
        self.browse.add_dialog = Some(DictEntryDraft::default());
    }

    /// 关闭新增词条对话框（放弃草稿）。
    pub fn browse_add_cancel(&mut self) {
        self.browse.add_dialog = None;
    }

    /// 新增对话框：词。
    pub fn browse_add_word(&mut self, value: String) {
        if let Some(draft) = self.browse.add_dialog.as_mut() {
            draft.word = value;
        }
    }

    /// 新增对话框：编码。
    pub fn browse_add_code(&mut self, value: String) {
        if let Some(draft) = self.browse.add_dialog.as_mut() {
            draft.code = value;
        }
    }

    /// 新增对话框：频率。
    pub fn browse_add_commits(&mut self, value: String) {
        if let Some(draft) = self.browse.add_dialog.as_mut() {
            draft.commits = value;
        }
    }

    /// 提交新增词条：本地校验（服务端还会再校验一遍）→ 写入线程。
    ///
    /// 写入在服务端是"销毁会话 → 导入一行 → 重建会话"，会打断正在输入的句子
    /// ——提示文案里说清楚。
    pub fn browse_add_submit(&mut self) {
        if self.browse.writing {
            return;
        }
        let Some(draft) = self.browse.add_dialog.clone() else {
            return;
        };
        let word = draft.word.trim().to_string();
        let code = draft.code.trim().to_string();
        if word.is_empty() || code.is_empty() {
            self.browse.error = Some("词和编码都要填".to_string());
            return;
        }
        for (label, value) in [("词", &word), ("编码", &code)] {
            if value.contains('\t') || value.contains('\n') || value.contains('\r') {
                self.browse.error = Some(format!("{label}不能含制表符或换行"));
                return;
            }
        }
        let commits = match parse_commits_input(&draft.commits) {
            Ok(v) => v,
            Err(reason) => {
                self.browse.error = Some(reason);
                return;
            }
        };
        self.browse.add_dialog = None;
        self.browse.error = None;
        self.start_entry_write(word, code, commits);
    }

    /// 请求删除词条（两步确认第一步）。
    pub fn browse_delete_request(&mut self, word: String, code: String) {
        if !self.browse.writing {
            self.browse.delete_confirm = Some((word, code));
        }
    }

    /// 取消删除。
    pub fn browse_delete_cancel(&mut self) {
        self.browse.delete_confirm = None;
    }

    /// 确认删除：写一行频率 -1 的删除标记。
    ///
    /// tombstone 不是物理删除——被删的词之后再次被输入并选中会"复活"；
    /// 频率也调不低（librime 的导入合并语义取较大频率）。
    pub fn browse_delete_confirm(&mut self, word: String, code: String) {
        self.browse.delete_confirm = None;
        self.start_entry_write(word, code, -1);
    }

    /// 发起一次词条写入（新增 commits>0 / 删除 commits<0），结果走
    /// `DICT_WRITE_OUTCOME`（与读取信箱分开，避免互相覆盖）。
    fn start_entry_write(&mut self, word: String, code: String, commits: i32) {
        if self.browse.writing {
            return;
        }
        self.browse.writing = true;
        self.browse.notice = None;
        self.browse.error = None;
        let dict = self.browse.dict.clone();
        std::thread::spawn(move || {
            let result = match notify_dict_entry_write(&dict, &word, &code, commits) {
                Some(n) if n > 0 => DictWriteResult {
                    dict: dict.clone(),
                    ok: true,
                    message: if commits > 0 {
                        "已添加词条（写入会打断正在输入的句子）".to_string()
                    } else {
                        "已标记删除（该词之后再次被输入并选中会复活）".to_string()
                    },
                },
                Some(_) => DictWriteResult {
                    dict: dict.clone(),
                    ok: false,
                    message: "写入词条失败（librime 写入 0 条，词库可能正被占用）".to_string(),
                },
                None => DictWriteResult {
                    dict: dict.clone(),
                    ok: false,
                    message: "写入词条失败（输入法服务未运行？）".to_string(),
                },
            };
            *DICT_WRITE_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        });
    }

    // ---- 快捷短语（词典页 Tab 1：按方案编辑 custom_phrase.txt） ----

    /// 进入快捷短语 Tab 时打开（schema_id + 方案显示名），立刻读一次。
    ///
    /// Tab 离开后状态保留（Tab 间切换不丢已编辑的本地表）；换方案走
    /// `phrase_schema_changed`。
    pub fn phrase_open(&mut self, schema_id: String, schema_name: String) {
        self.message = None;
        self.phrase = Some(CustomPhraseState {
            schema_id,
            schema_name,
            ..CustomPhraseState::default()
        });
        self.start_phrase_fetch();
    }

    /// 切换目标方案（下拉）：清状态重读。
    pub fn phrase_schema_changed(&mut self, schema_id: String, schema_name: String) {
        let Some(phrase) = self.phrase.as_mut() else {
            return;
        };
        if phrase.schema_id == schema_id {
            return;
        }
        phrase.schema_id = schema_id;
        phrase.schema_name = schema_name;
        phrase.entries.clear();
        phrase.loaded = false;
        phrase.loading = false;
        phrase.saving = false;
        phrase.error = None;
        phrase.notice = None;
        phrase.delete_confirm = None;
        phrase.dialog = None;
        self.start_phrase_fetch();
    }

    /// 打开新增短语对话框。
    pub fn phrase_add_open(&mut self) {
        if let Some(phrase) = self.phrase.as_mut() {
            if !phrase.saving {
                phrase.notice = None;
                phrase.error = None;
                phrase.dialog = Some(PhraseDialogState::default());
            }
        }
    }

    /// 编辑第 i 条（打开对话框并预填）。
    pub fn phrase_edit(&mut self, index: usize) {
        if let Some(phrase) = self.phrase.as_mut() {
            if phrase.saving {
                return;
            }
            if let Some(entry) = phrase.entries.get(index) {
                phrase.notice = None;
                phrase.error = None;
                phrase.dialog = Some(PhraseDialogState {
                    editing: Some(index),
                    word: entry.word.clone(),
                    code: entry.code.clone(),
                    weight: entry.weight.map(|w| w.to_string()).unwrap_or_default(),
                });
            }
        }
    }

    /// 关闭短语对话框（放弃草稿）。
    pub fn phrase_dialog_cancel(&mut self) {
        if let Some(phrase) = self.phrase.as_mut() {
            phrase.dialog = None;
        }
    }

    /// 对话框：词。
    pub fn phrase_dialog_word(&mut self, value: String) {
        if let Some(phrase) = self.phrase.as_mut() {
            if let Some(dialog) = phrase.dialog.as_mut() {
                dialog.word = value;
            }
        }
    }

    /// 对话框：编码。
    pub fn phrase_dialog_code(&mut self, value: String) {
        if let Some(phrase) = self.phrase.as_mut() {
            if let Some(dialog) = phrase.dialog.as_mut() {
                dialog.code = value;
            }
        }
    }

    /// 对话框：权重。
    pub fn phrase_dialog_weight(&mut self, value: String) {
        if let Some(phrase) = self.phrase.as_mut() {
            if let Some(dialog) = phrase.dialog.as_mut() {
                dialog.weight = value;
            }
        }
    }

    /// 提交对话框（新增或编辑）：改本地整表 → 整表保存到 server。
    pub fn phrase_dialog_submit(&mut self) {
        let Some(phrase) = self.phrase.as_mut() else {
            return;
        };
        if phrase.saving {
            return;
        }
        let Some(dialog) = phrase.dialog.clone() else {
            return;
        };
        let word = dialog.word.trim().to_string();
        let code = dialog.code.trim().to_string();
        if word.is_empty() || code.is_empty() {
            phrase.error = Some("词和编码都要填".to_string());
            return;
        }
        for (label, value) in [("词", &word), ("编码", &code)] {
            if value.contains('\t') || value.contains('\n') || value.contains('\r') {
                phrase.error = Some(format!("{label}不能含制表符或换行"));
                return;
            }
        }
        let weight = match parse_weight_input(&dialog.weight) {
            Ok(v) => v,
            Err(reason) => {
                phrase.error = Some(reason);
                return;
            }
        };
        let row = CustomPhraseRow { word, code, weight };
        match dialog.editing {
            Some(index) => {
                if index < phrase.entries.len() {
                    phrase.entries[index] = row;
                }
            }
            None => phrase.entries.push(row),
        }
        phrase.dialog = None;
        phrase.error = None;
        self.start_phrase_save();
    }

    /// 请求删除第 i 条（两步确认第一步）。
    pub fn phrase_delete_request(&mut self, index: usize) {
        if let Some(phrase) = self.phrase.as_mut() {
            if !phrase.saving && index < phrase.entries.len() {
                phrase.delete_confirm = Some(index);
            }
        }
    }

    /// 取消删除。
    pub fn phrase_delete_cancel(&mut self) {
        if let Some(phrase) = self.phrase.as_mut() {
            phrase.delete_confirm = None;
        }
    }

    /// 确认删除：从本地整表移除并整表保存。
    pub fn phrase_delete_confirm(&mut self, index: usize) {
        let Some(phrase) = self.phrase.as_mut() else {
            return;
        };
        if index < phrase.entries.len() {
            phrase.entries.remove(index);
        }
        phrase.delete_confirm = None;
        self.start_phrase_save();
    }

    /// 发起短语表读取（在途时忽略——短语表小，重读快）。
    fn start_phrase_fetch(&mut self) {
        let Some(phrase) = self.phrase.as_mut() else {
            return;
        };
        if phrase.loading {
            return;
        }
        phrase.loading = true;
        phrase.error = None;
        let schema_id = phrase.schema_id.clone();
        std::thread::spawn(move || {
            let outcome = match notify_phrase_list(&schema_id) {
                Some(result) => PhraseTaskResult::Loaded { schema_id, result },
                None => PhraseTaskResult::Failed {
                    schema_id,
                    reason: "读取快捷短语失败（输入法服务未运行？）".to_string(),
                },
            };
            *PHRASE_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        });
    }

    /// 整表保存当前短语表（server 写文件 + 视需要注入方案 patch，不做部署）。
    fn start_phrase_save(&mut self) {
        let Some(phrase) = self.phrase.as_mut() else {
            return;
        };
        if phrase.saving {
            return;
        }
        phrase.saving = true;
        phrase.notice = None;
        phrase.error = None;
        let schema_id = phrase.schema_id.clone();
        let entries = phrase.entries.clone();
        std::thread::spawn(move || {
            let outcome = match notify_phrase_save(&schema_id, &entries) {
                Some(result) => PhraseTaskResult::Saved { schema_id, result },
                None => PhraseTaskResult::Failed {
                    schema_id,
                    reason: "保存快捷短语失败（输入法服务未运行？）".to_string(),
                },
            };
            *PHRASE_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        });
    }

    /// BackgroundPoll 节拍：取后台任务结果 + 处理关键词防抖。
    pub fn poll(&mut self) {
        let outcome = DICT_TASK_OUTCOME
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match outcome {
            Some(DictTaskResult::List(r)) => {
                self.dicts = r.dicts;
                self.sync_dir = r.sync_dir;
                self.busy = None;
                // 还没选中词典时自动选第一本（对齐小狼毫：打开就有内容可看，
                // 不该让用户先在下拉里点一下）。browse_select 会触发首读。
                if self.browse.dict.is_empty() {
                    if let Some(first) = self.dicts.first().cloned() {
                        self.browse_select(first);
                    }
                }
            }
            Some(DictTaskResult::Message(m)) => {
                self.busy = None;
                self.message = Some(m);
            }
            Some(DictTaskResult::Entries { dict, result }) if self.browse.dict == dict => {
                // 词典名对不上 = 在途期间切换过下拉，旧结果丢弃。
                self.browse.loading = false;
                self.browse.loaded = true;
                self.browse.total = result.total;
                self.browse.matched = result.matched;
                self.browse.entries = result.entries;
                self.browse.page = 0;
            }
            Some(DictTaskResult::EntriesFailed { dict, reason }) if self.browse.dict == dict => {
                self.browse.loading = false;
                self.browse.loaded = true;
                self.browse.error = Some(reason);
            }
            _ => {}
        }

        // 词条写入结果：成功给提示 + 排一次防抖后重读（词库内容变了）。
        // 词典名对不上（期间切过下拉）时只清 writing——结果留给旧词典，
        // 不往新词典的视图上贴。
        let write_outcome = DICT_WRITE_OUTCOME
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(result) = write_outcome {
            self.browse.writing = false;
            if result.dict == self.browse.dict {
                if result.ok {
                    self.browse.notice = Some(result.message);
                    self.browse.pending = Some(std::time::Instant::now());
                } else {
                    self.browse.error = Some(result.message);
                }
            }
        }

        // 快捷短语结果（读取/保存共用一个信箱；schema_id 对不上 = 期间换过方案，
        // 丢弃；子视图已关则丢弃）。
        let phrase_outcome = PHRASE_OUTCOME
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match phrase_outcome {
            Some(PhraseTaskResult::Loaded { schema_id, result }) => {
                if let Some(phrase) = self.phrase.as_mut() {
                    if phrase.schema_id == schema_id {
                        phrase.loading = false;
                        phrase.loaded = true;
                        phrase.dict_name = result.dict_name;
                        phrase.file_name = result.file_name;
                        phrase.file_exists = result.file_exists;
                        phrase.patch_applied = result.patch_applied;
                        phrase.entries = result.entries;
                    }
                }
            }
            Some(PhraseTaskResult::Saved { schema_id, result }) => {
                if let Some(phrase) = self.phrase.as_mut() {
                    if phrase.schema_id == schema_id {
                        phrase.saving = false;
                        phrase.loaded = true;
                        phrase.dict_name = result.dict_name;
                        phrase.file_name = result.file_name;
                        phrase.file_exists = result.file_exists;
                        phrase.patch_applied = result.patch_applied;
                        if result.patch_added {
                            phrase.patch_added = true;
                        }
                        phrase.entries = result.entries;
                        phrase.notice = Some(if result.patch_added {
                            "已保存；首次启用快捷短语要重新部署方案（输入方案页 → 部署方案）"
                                .to_string()
                        } else {
                            "已保存（重新部署方案后生效）".to_string()
                        });
                    }
                }
            }
            Some(PhraseTaskResult::Failed { schema_id, reason }) => {
                if let Some(phrase) = self.phrase.as_mut() {
                    if phrase.schema_id == schema_id {
                        phrase.loading = false;
                        phrase.saving = false;
                        phrase.error = Some(reason);
                    }
                }
            }
            None => {}
        }

        // 关键词改动（或写词条后的补读）过了防抖就补一次读取。
        let due = self
            .browse
            .pending
            .map(|since| since.elapsed() >= DICT_QUERY_DEBOUNCE)
            .unwrap_or(false);
        if due {
            self.start_entries_fetch();
        }
    }
}

/// 新增词条的频率输入：空串 = 1；否则必须是正整数。
#[cfg(any(windows, feature = "dict-page"))]
fn parse_commits_input(raw: &str) -> Result<i32, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(1);
    }
    let value: i32 = trimmed
        .parse()
        .map_err(|_| "频率要填正整数（留空即 1）".to_string())?;
    if value <= 0 {
        return Err("频率要填正整数（留空即 1）".to_string());
    }
    Ok(value)
}

/// 短语权重输入：空串 = 省略该列（None）；否则必须是正整数。
#[cfg(any(windows, feature = "dict-page"))]
fn parse_weight_input(raw: &str) -> Result<Option<i32>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: i32 = trimmed
        .parse()
        .map_err(|_| "权重要是正整数（留空走默认）".to_string())?;
    if value <= 0 {
        return Err("权重要是正整数（留空走默认）".to_string());
    }
    Ok(Some(value))
}

/// 剪贴板同步插件的页面状态（开关 + 选择 + 配置表单）。
#[cfg(feature = "clipboard-page")]
#[derive(Clone)]
pub struct SyncPluginUiState {
    /// 是否启用插件同步（写入 clipboard_sync.toml，IME 常驻循环热加载）。
    pub enabled: bool,
    /// 已安装的 clipboard_sync 插件。
    pub plugins: Vec<ClipboardSyncPluginInfo>,
    /// 当前选中的插件下标。
    pub plugin_index: usize,
    /// 插件配置表单（schema + 当前值）。
    pub fields: Vec<(xime_plugin::SettingField, String)>,
    pub message: Option<String>,
    pub busy: Option<&'static str>,
}

#[cfg(feature = "clipboard-page")]
static SYNC_PLUGIN_OUTCOME: std::sync::Mutex<Option<Result<SyncPluginOutcome, String>>> =
    std::sync::Mutex::new(None);

#[cfg(feature = "clipboard-page")]
enum SyncPluginOutcome {
    SchemaLoaded(Vec<xime_plugin::SettingField>, Vec<(String, String)>),
    Tested,
}

#[cfg(feature = "clipboard-page")]
impl Default for SyncPluginUiState {
    fn default() -> Self {
        let plugins = scan_clipboard_sync_plugins();
        let cfg = read_clipboard_sync_config();
        let plugin_index = plugins
            .iter()
            .position(|p| p.id == cfg.plugin_id)
            .unwrap_or(0);
        let mut st = Self {
            enabled: cfg.enabled,
            plugins,
            plugin_index,
            fields: Vec::new(),
            message: None,
            busy: None,
        };
        st.start_schema_load();
        st
    }
}

#[cfg(feature = "clipboard-page")]
impl SyncPluginUiState {
    fn selected(&self) -> Option<&ClipboardSyncPluginInfo> {
        self.plugins.get(self.plugin_index)
    }

    /// 后台加载当前插件的 schema 与配置值。
    fn start_schema_load(&mut self) {
        let Some((id, dir)) = self.selected().map(|i| (i.id.clone(), i.dir.clone())) else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("schema");
        self.fields.clear();
        std::thread::spawn(move || {
            let result = load_plugin_runtime(&dir, &id).map(|rt| {
                let fields = rt.get_settings_schema();
                let content = std::fs::read_to_string(
                    config_base_dir()
                        .join("plugins/config")
                        .join(format!("{id}.yaml")),
                )
                .unwrap_or_default();
                let mut config: std::collections::BTreeMap<String, String> =
                    serde_yaml::from_str(&content).unwrap_or_default();
                let key_path = xime_plugin::cipher::key_path_for_config(
                    &config_base_dir()
                        .join("plugins/config")
                        .join(format!("{id}.yaml")),
                );
                for value in config.values_mut() {
                    if let Some(plain) =
                        xime_plugin::cipher::decrypt_with_key_path(&key_path, value)
                    {
                        *value = plain;
                    }
                }
                let values = fields
                    .iter()
                    .map(|f| {
                        (
                            f.key.clone(),
                            config.get(&f.key).cloned().unwrap_or_default(),
                        )
                    })
                    .collect();
                SyncPluginOutcome::SchemaLoaded(fields, values)
            });
            *SYNC_PLUGIN_OUTCOME.lock().unwrap() = Some(result);
        });
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        let mut cfg = read_clipboard_sync_config();
        cfg.enabled = enabled;
        write_clipboard_sync_config(&cfg);
        notify_daemon_reload_plugins();
        self.message = Some(if enabled {
            "插件同步已启用".to_string()
        } else {
            "插件同步已停用".to_string()
        });
    }

    pub fn select(&mut self, index: usize) {
        if index >= self.plugins.len() || index == self.plugin_index {
            return;
        }
        self.plugin_index = index;
        self.message = None;
        let id = self.plugins[index].id.clone();
        let mut cfg = read_clipboard_sync_config();
        cfg.plugin_id = id.clone();
        write_clipboard_sync_config(&cfg);
        // 对齐 Android：选中的同步插件自动启用（daemon 仅加载 plugin_id 一项）。
        let manager = xime_plugin::PluginManager::new(config_base_dir().join("plugins"));
        if let Err(e) = manager.set_enabled(&id, true) {
            let not_found = matches!(
                e,
                xime_plugin::manager::ManagerError::Io(ref io)
                    if io.kind() == std::io::ErrorKind::NotFound
            );
            // 无注册记录的目录直装插件默认视为启用，NotFound 可忽略。
            if !not_found {
                self.message = Some(format!("启用插件失败: {e}"));
            }
        }
        notify_daemon_reload_plugins();
        self.start_schema_load();
    }

    pub fn set_field(&mut self, key: String, value: String) {
        let Some(id) = self.selected().map(|i| i.id.clone()) else {
            return;
        };
        for (field, current) in self.fields.iter_mut() {
            if field.key == key {
                *current = value.clone();
            }
        }
        let path = config_base_dir()
            .join("plugins/config")
            .join(format!("{id}.yaml"));
        let existing: std::collections::BTreeMap<String, String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_yaml::from_str(&c).ok())
            .unwrap_or_default();
        let mut config = existing;
        config.insert(key, value);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(yaml) = serde_yaml::to_string(&config) {
            let _ = std::fs::write(path, yaml);
        }
        // 配置保存 ≠ 启用同步：开关未开时明确提示，避免「配了但不推送」的困惑。
        if !self.enabled {
            self.message =
                Some("配置已保存；当前同步未启用，请先打开「启用剪贴板同步」开关".to_string());
        }
    }

    pub fn start_test(&mut self) {
        let Some((id, dir)) = self.selected().map(|i| (i.id.clone(), i.dir.clone())) else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("test");
        self.message = None;
        std::thread::spawn(move || {
            let result = load_plugin_runtime(&dir, &id).and_then(|rt| match rt.test_connection() {
                Some(msg) => Err(msg),
                None => Ok(SyncPluginOutcome::Tested),
            });
            *SYNC_PLUGIN_OUTCOME.lock().unwrap() = Some(result);
        });
    }

    /// 轮询后台结果。
    pub fn poll(&mut self) {
        let outcome = SYNC_PLUGIN_OUTCOME.lock().unwrap().take();
        let Some(outcome) = outcome else {
            return;
        };
        self.busy = None;
        match outcome {
            Ok(SyncPluginOutcome::SchemaLoaded(fields, values)) => {
                let map: std::collections::HashMap<String, String> = values.into_iter().collect();
                self.fields = fields
                    .into_iter()
                    .map(|f| {
                        let v = map.get(&f.key).cloned().unwrap_or_default();
                        (f, v)
                    })
                    .collect();
            }
            Ok(SyncPluginOutcome::Tested) => {
                self.message = Some("连接成功".to_string());
            }
            Err(e) => {
                self.message = Some(e);
            }
        }
    }
}

/// 生成随机认证密码（16 字节随机 → URL 安全 base64）。
#[cfg(feature = "clipboard-page")]
fn random_password() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("getrandom");
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

// ---- 云备份（backup-page） --------------------------------------------------

/// 备份提供者：内置 WebDAV 或 backup 类型插件（与 Android 同契约）。
#[cfg(feature = "backup-page")]
#[derive(Clone, Debug, PartialEq)]
pub enum BackupProvider {
    Builtin,
    Plugin {
        id: String,
        name: String,
        dir: std::path::PathBuf,
    },
}

#[cfg(feature = "backup-page")]
impl BackupProvider {
    pub fn label(&self) -> &str {
        match self {
            Self::Builtin => "内置 WebDAV",
            Self::Plugin { name, .. } => name,
        }
    }
}

/// 远端备份列表条目（内置/插件统一展示模型）。
#[cfg(feature = "backup-page")]
#[derive(Clone, Debug, PartialEq)]
pub struct BackupEntry {
    /// 操作 id：内置 = 服务器路径；插件 = 插件定义的 id。
    pub id: String,
    pub name: String,
    pub size: Option<i64>,
}

/// 云备份页状态：提供者选择 + WebDAV（内置）配置 + 备份模式 + 后台操作进度。
#[cfg(feature = "backup-page")]
#[derive(Clone)]
pub struct BackupState {
    /// 可用提供者（内置 WebDAV 恒在，后接 backup 插件）。
    pub providers: Vec<BackupProvider>,
    /// 当前选中的提供者下标。
    pub provider: usize,
    /// 插件配置表单字段（schema + 当前值），选中插件提供者后由后台线程填充。
    pub plugin_fields: Vec<(xime_plugin::SettingField, String)>,
    /// WebDAV 服务器地址（仅内置提供者使用）。
    pub url: String,
    pub username: String,
    /// WebDAV 密码（写入本地配置文件，权限 0600）。
    pub password: String,
    /// 远端备份目录（相对 WebDAV 根路径）。
    pub remote_dir: String,
    /// 备份模式（0=仅配置, 1=全量）。
    pub mode: u8,
    /// 进行中的操作标识（test/schema/backup/list/restore/delete）。
    pub busy: Option<&'static str>,
    /// 最近一次操作的状态消息。
    pub message: Option<String>,
    /// 远端备份列表（"查看远端备份"后填充，切换提供者时清空）。
    pub remote: Vec<BackupEntry>,
    /// 配置文件路径（backup.toml，仅内置提供者的连接信息）。
    config_path: std::path::PathBuf,
}

/// 云备份后台操作的完成结果（线程 → 轮询回 UI）。
#[cfg(feature = "backup-page")]
enum BackupOutcome {
    Tested,
    SchemaLoaded(Vec<xime_plugin::SettingField>, Vec<(String, String)>),
    BackedUp(String),
    Listed(Vec<BackupEntry>),
    Restored(usize),
    Deleted,
}

#[cfg(feature = "backup-page")]
static BACKUP_OUTCOME: std::sync::Mutex<Option<Result<BackupOutcome, String>>> =
    std::sync::Mutex::new(None);

#[cfg(feature = "backup-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct BackupConfigFile {
    webdav: BackupWebDavSection,
}

#[cfg(feature = "backup-page")]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct BackupWebDavSection {
    url: Option<String>,
    username: Option<String>,
    password: Option<String>,
    remote_dir: Option<String>,
}

#[cfg(any(feature = "backup-page", feature = "clipboard-page"))]
fn backup_plugins_root() -> std::path::PathBuf {
    config_base_dir().join("plugins")
}

/// 扫描 backup 类型且启用的插件（顺序按目录名，稳定）。
#[cfg(feature = "backup-page")]
fn scan_backup_plugins() -> Vec<BackupProvider> {
    let root = backup_plugins_root();
    let manager = xime_plugin::PluginManager::new(&root);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let Ok(manifest) = xime_plugin::PluginManifest::from_dir(&dir) else {
            continue;
        };
        if manifest.plugin_type() != xime_plugin::PluginType::Backup {
            continue;
        }
        let enabled = manager.get(&manifest.id).map(|r| r.enabled).unwrap_or(true);
        if !enabled {
            continue;
        }
        out.push(BackupProvider::Plugin {
            id: manifest.id,
            name: manifest.name,
            dir,
        });
    }
    out
}

/// 插件配置文件（host.config 同一文件：plugins/config/<id>.yaml）。
#[cfg(any(feature = "backup-page", feature = "clipboard-page"))]
fn plugin_config_path(id: &str) -> std::path::PathBuf {
    backup_plugins_root()
        .join("config")
        .join(format!("{id}.yaml"))
}

#[cfg(any(feature = "backup-page", feature = "clipboard-page"))]
fn read_plugin_config(id: &str) -> std::collections::BTreeMap<String, String> {
    let path = plugin_config_path(id);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Default::default();
    };
    let mut map: std::collections::BTreeMap<String, String> =
        serde_yaml::from_str(&content).unwrap_or_default();
    // 值解密（host.config 同一密文格式；无前缀旧版明文原样）
    let key_path = xime_plugin::cipher::key_path_for_config(&path);
    map.retain(
        |k, v| match xime_plugin::cipher::decrypt_with_key_path(&key_path, v) {
            Some(plain) => {
                *v = plain;
                true
            }
            None => {
                tracing::warn!("[config] 值解密失败，条目按缺失处理: {k}");
                false
            }
        },
    );
    map
}

#[cfg(any(feature = "backup-page", feature = "clipboard-page"))]
fn write_plugin_config(id: &str, map: &std::collections::BTreeMap<String, String>) {
    let path = plugin_config_path(id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 值加密落盘（与 host.config 同一密文格式；明文旧值随全量写入自动升级）
    let key_path = xime_plugin::cipher::key_path_for_config(&path);
    let encrypted: std::collections::BTreeMap<String, String> = map
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                xime_plugin::cipher::encrypt_with_key_path(&key_path, v),
            )
        })
        .collect();
    if let Ok(yaml) = serde_yaml::to_string(&encrypted) {
        let _ = std::fs::write(path, yaml);
    }
}

/// 按需加载插件运行时（一操作一实例，onLoad 幂等）。
#[cfg(any(feature = "backup-page", feature = "clipboard-page"))]
fn load_plugin_runtime(
    dir: &std::path::Path,
    id: &str,
) -> Result<xime_plugin::PluginRuntime, String> {
    let manifest = xime_plugin::PluginManifest::from_dir(dir)
        .map_err(|e| format!("读取 manifest 失败: {e}"))?;
    let runtime = xime_plugin::PluginRuntime::load(dir, &manifest.entry, &plugin_config_path(id))
        .map_err(|e| format!("加载插件失败: {e}"))?;
    runtime.call_on_load();
    Ok(runtime)
}

#[cfg(feature = "backup-page")]
impl Default for BackupState {
    fn default() -> Self {
        let providers = scan_backup_plugins();
        let mut st = Self {
            providers,
            provider: 0,
            plugin_fields: Vec::new(),
            url: String::new(),
            username: String::new(),
            password: String::new(),
            remote_dir: "xime-backup".to_string(),
            mode: 0,
            busy: None,
            message: None,
            remote: Vec::new(),
            config_path: config_base_dir().join("backup.toml"),
        };
        st.read_config();
        st
    }
}

#[cfg(feature = "backup-page")]
impl BackupState {
    fn read_config(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.config_path) else {
            return;
        };
        let Ok(cfg) = toml::from_str::<BackupConfigFile>(&content) else {
            return;
        };
        if let Some(v) = cfg.webdav.url {
            self.url = v;
        }
        if let Some(v) = cfg.webdav.username {
            self.username = v;
        }
        if let Some(v) = cfg.webdav.password {
            self.password = v;
        }
        if let Some(v) = cfg.webdav.remote_dir {
            self.remote_dir = v;
        }
    }

    /// 保存内置 WebDAV 配置（0600，密码仅本地可读）。
    fn write_config(&self) -> Result<(), String> {
        let cfg = BackupConfigFile {
            webdav: BackupWebDavSection {
                url: Some(self.url.clone()),
                username: Some(self.username.clone()),
                password: Some(self.password.clone()),
                remote_dir: Some(self.remote_dir.clone()),
            },
        };
        let content = toml::to_string(&cfg).map_err(|e| e.to_string())?;
        let parent = self.config_path.parent().ok_or("配置目录无效")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // 含明文密码：必须 0600 一步创建——先 0644 写入再补 chmod 存在同机
        // 其他用户可读的窗口。
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.config_path)
                .map_err(|e| e.to_string())?;
            f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.config_path, content).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn client(&self) -> Result<crate::webdav::WebDavClient, String> {
        if self.url.trim().is_empty() {
            return Err("请先填写 WebDAV 服务器地址".to_string());
        }
        Ok(crate::webdav::WebDavClient::new(
            self.url.trim().to_string(),
            (!self.username.is_empty()).then_some(self.username.clone()),
            (!self.password.is_empty()).then_some(self.password.clone()),
        ))
    }

    /// 当前提供者是否为插件。
    pub fn is_plugin_selected(&self) -> bool {
        matches!(
            self.providers.get(self.provider),
            Some(BackupProvider::Plugin { .. })
        )
    }

    /// 切换提供者：清空列表；插件提供者则后台加载 schema + 配置值。
    pub fn select_provider(&mut self, index: usize) {
        if index >= self.providers.len() || index == self.provider {
            return;
        }
        self.provider = index;
        self.remote.clear();
        self.plugin_fields.clear();
        self.message = None;
        let Some(BackupProvider::Plugin { id, dir, .. }) = self.providers.get(index) else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("schema");
        let id = id.clone();
        let dir = dir.clone();
        std::thread::spawn(move || {
            let result = load_plugin_runtime(&dir, &id).map(|rt| {
                let fields = rt.get_settings_schema();
                let config = read_plugin_config(&id);
                let values = fields
                    .iter()
                    .map(|f| {
                        (
                            f.key.clone(),
                            config.get(&f.key).cloned().unwrap_or_default(),
                        )
                    })
                    .collect();
                BackupOutcome::SchemaLoaded(fields, values)
            });
            *BACKUP_OUTCOME.lock().unwrap() = Some(result);
        });
    }

    /// 更新插件配置字段（写 host.config 同一文件，插件立即可见）。
    pub fn set_plugin_field(&mut self, key: String, value: String) {
        let Some(BackupProvider::Plugin { id, .. }) = self.providers.get(self.provider) else {
            return;
        };
        let id = id.clone();
        for (field, current) in self.plugin_fields.iter_mut() {
            if field.key == key {
                *current = value.clone();
            }
        }
        let mut config = read_plugin_config(&id);
        config.insert(key, value);
        write_plugin_config(&id, &config);
    }

    /// 通用后台任务启动：守卫 busy → 落盘配置 → 开线程执行 → 结果进轮询槽。
    fn start_op(
        &mut self,
        tag: &'static str,
        op: impl FnOnce(&BackupState) -> Result<BackupOutcome, String> + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        if let Err(e) = self.write_config() {
            self.message = Some(format!("保存配置失败: {e}"));
            return;
        }
        self.busy = Some(tag);
        self.message = None;
        let snapshot = self.clone();
        std::thread::spawn(move || {
            // catch_unwind：op 里跑的是插件/第三方代码（load_plugin_runtime），
            // panic 不落结果信箱的话 busy 永不清除，备份页所有入口永久卡死。
            let result =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| op(&snapshot))) {
                    Ok(r) => r,
                    Err(_) => Err("后台任务异常终止（panic），请重试".to_string()),
                };
            *BACKUP_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        });
    }

    pub fn start_test(&mut self) {
        self.start_op("test", |state| match state.providers.get(state.provider) {
            Some(BackupProvider::Plugin { id, dir, .. }) => {
                let runtime = load_plugin_runtime(dir, id)?;
                if let Some(msg) = runtime.test_connection() {
                    return Err(msg);
                }
                Ok(BackupOutcome::Tested)
            }
            _ => {
                state.client()?.test()?;
                Ok(BackupOutcome::Tested)
            }
        });
    }

    pub fn start_backup(&mut self) {
        self.start_op("backup", |state| {
            let mode = crate::backup::BackupMode::from_index(state.mode).ok_or("备份模式无效")?;
            let (_, user_dir) = get_data_dirs();
            let archive = crate::backup::pack_rime(&user_dir, mode)?;
            let name = crate::backup::archive_name(
                mode,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
            );
            match state.providers.get(state.provider) {
                Some(BackupProvider::Plugin { id, dir, .. }) => {
                    let runtime = load_plugin_runtime(dir, id)?;
                    let result = runtime.push_backup(&name, &archive);
                    if !result.ok {
                        return Err(result.message.unwrap_or_else(|| "上传失败".to_string()));
                    }
                }
                _ => {
                    let key = format!("{}/{}", state.remote_dir.trim_matches('/'), name);
                    state.client()?.put(&key, &archive)?;
                }
            }
            Ok(BackupOutcome::BackedUp(name))
        });
    }

    pub fn start_list(&mut self) {
        self.start_op("list", |state| match state.providers.get(state.provider) {
            Some(BackupProvider::Plugin { id, dir, .. }) => {
                let runtime = load_plugin_runtime(dir, id)?;
                let list = runtime.list_backups().ok_or("获取备份列表失败")?;
                Ok(BackupOutcome::Listed(
                    list.into_iter()
                        .map(|e| BackupEntry {
                            id: e.id,
                            name: e.name,
                            size: Some(e.size),
                        })
                        .collect(),
                ))
            }
            _ => {
                let dir = state.remote_dir.trim_matches('/').to_string();
                Ok(BackupOutcome::Listed(
                    state
                        .client()?
                        .list(&dir)?
                        .into_iter()
                        .map(|f| BackupEntry {
                            id: f.path,
                            name: f.name,
                            size: f.size.map(|s| s as i64),
                        })
                        .collect(),
                ))
            }
        });
    }

    pub fn start_restore(&mut self, id: String) {
        self.start_op("restore", move |state| {
            let data = match state.providers.get(state.provider) {
                Some(BackupProvider::Plugin { id: pid, dir, .. }) => {
                    let runtime = load_plugin_runtime(dir, pid)?;
                    runtime
                        .pull_backup(&id)
                        .ok_or_else(|| "下载备份包失败".to_string())?
                }
                _ => state
                    .client()?
                    .get(&id)?
                    .ok_or_else(|| "远端备份不存在".to_string())?,
            };
            let (_, user_dir) = get_data_dirs();
            let n = crate::backup::unpack_rime(&data, &user_dir)?;
            Ok(BackupOutcome::Restored(n))
        });
    }

    pub fn start_delete(&mut self, id: String) {
        self.start_op("delete", move |state| {
            match state.providers.get(state.provider) {
                Some(BackupProvider::Plugin { id: pid, dir, .. }) => {
                    let runtime = load_plugin_runtime(dir, pid)?;
                    if !runtime.delete_backup(&id) {
                        return Err("删除失败".to_string());
                    }
                }
                _ => {
                    state.client()?.delete(&id)?;
                }
            }
            Ok(BackupOutcome::Deleted)
        });
    }

    /// 轮询后台结果（由 poll_background 调用）。
    pub fn poll(&mut self) {
        let outcome = BACKUP_OUTCOME.lock().unwrap().take();
        let Some(outcome) = outcome else {
            return;
        };
        self.busy = None;
        match outcome {
            Ok(BackupOutcome::Tested) => {
                self.message = Some("连接成功".to_string());
            }
            Ok(BackupOutcome::SchemaLoaded(fields, values)) => {
                let map: std::collections::HashMap<String, String> = values.into_iter().collect();
                self.plugin_fields = fields
                    .into_iter()
                    .map(|f| {
                        let v = map.get(&f.key).cloned().unwrap_or_default();
                        (f, v)
                    })
                    .collect();
            }
            Ok(BackupOutcome::BackedUp(name)) => {
                self.message = Some(format!("备份完成：{name}"));
            }
            Ok(BackupOutcome::Listed(files)) => {
                self.message = Some(format!("共 {} 个远端备份", files.len()));
                self.remote = files;
            }
            Ok(BackupOutcome::Restored(n)) => {
                self.message = Some(format!("恢复完成（{n} 个文件），重启输入法后生效"));
            }
            Ok(BackupOutcome::Deleted) => {
                self.message = Some("已删除".to_string());
                // 删除后自动刷新列表。
                self.start_list();
            }
            Err(e) => {
                self.message = Some(e);
            }
        }
    }
}

// ---- 云备份（backup-page）结束 ----------------------------------------------

#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
pub struct SyncState {
    pub url: String,
    pub username: String,
    pub password: String,
    pub is_syncing: bool,
    pub status: SyncStatus,
    pub status_message: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SyncStatus {
    #[default]
    Idle,
    Success,
    Error,
}

#[derive(Clone)]
pub struct AppearanceState {
    pub font_size: f64,
    pub candidate_count: i32,
    pub corner_radius: f64,
    pub color_scheme: ColorSchemeConfig,
    pub dark_mode: DarkMode,
    pub available_color_schemes: Vec<(String, String, u32)>,
    pub color_schemes_loaded: bool,
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            candidate_count: 5,
            corner_radius: 8.0,
            color_scheme: ColorSchemeConfig::default(),
            dark_mode: DarkMode::default(),
            available_color_schemes: Vec::new(),
            color_schemes_loaded: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct InputSchemaState {
    pub selected_schema: usize,
    pub available_schemas: Vec<SchemaInfo>,
    /// 与 `available_schemas` 一一对应的归属方案包 id（`builtin` = 内置方案包）。
    pub schema_packages: Vec<String>,
    pub schema_config: SchemaConfig,
    pub config_loaded: bool,
    pub current_tab: usize,
    /// 内置方案包已卸载但 `market/builtin/` 备份仍在 → 可一键还原。
    pub builtin_restorable: bool,
    /// `build/` 里有编译产物的方案 id（= 真正「已部署」的方案）。
    ///
    /// 目录里有 `*.schema.yaml` 不等于能用：librime 只编译启用列表里的方案，
    /// 没产物的方案切换时会被服务端按「未部署」拒绝（见 `RimeEngine::schema_deployed`）。
    /// 设置页据此把没产物的方案标成「未启用」。
    pub deployed_schema_ids: Vec<String>,
    /// 「方案词表」tab 的浏览状态（只读；schema_id 跟本页选中行同步）。
    #[cfg(any(windows, feature = "dict-page"))]
    pub dict: SchemaDictState,
}

/// 安装/还原前的冲突确认（对齐安卓 `SchemaLocalUiState.conflictPackageId` /
/// `conflictingSchemeIds`）：rime 目录已有其他方案包时先确认卸载。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaInstallConflict {
    /// 目标方案包 id；`builtin` 表示「还原内置方案包」（不是安装）。
    pub schema_id: String,
    /// 需要先卸载的方案包（含内置方案包 `builtin`）。
    pub packages: Vec<String>,
}

impl SchemaInstallConflict {
    /// 目标是否为「还原内置方案包」（而非安装市场包）。
    pub fn is_restore_builtin(&self) -> bool {
        self.schema_id == BUILTIN_PACKAGE_ID
    }
}

#[derive(Clone, Default)]
pub struct MarketSchemaState {
    pub schemas: Vec<MarketSchema>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub installed_ids: Vec<String>,
    pub downloaded_ids: Vec<String>,
    pub downloading: Option<String>,
    pub download_progress: Option<f32>,
    pub installing: Option<String>,
    pub install_message: Option<String>,
    pub install_message_since: Option<std::time::Instant>,
    /// 待确认的安装冲突（None = 无冲突确认弹窗）。
    pub conflict_install: Option<SchemaInstallConflict>,
    /// 「清理内置方案」二次确认（避免一键删掉内置文件）。
    pub builtin_clean_confirm: bool,
    /// 扩展商店当前 Tab（0=方案, 1=模型）。
    pub store_tab: usize,
    /// 分类筛选（None=全部）。
    pub selected_tag: Option<String>,
    /// 每个方案选中的版本。
    pub selected_versions: HashMap<String, String>,
    /// 索引更新时间。
    pub updated_at: String,
}

#[derive(Clone, Default)]
pub struct SchemaImportState {
    pub open: bool,
    pub path: String,
    pub url: String,
    pub package_id: String,
    pub busy: bool,
    /// 原生文件选择框是否正在后台打开（大目录时不能阻塞 UI）。
    pub browsing: bool,
    pub message: Option<String>,
    /// (已下载字节, 总字节[0=未知], 字节/秒)。
    pub progress: Option<(u64, u64, u64)>,
    /// 进度条动画帧（未知总大小时做往返扫描）。
    pub tick: u32,
}

impl SchemaImportState {
    pub fn open_dialog(&mut self) {
        self.open = true;
        self.message = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.busy = false;
        self.message = None;
    }

    pub fn set_path(&mut self, path: String) {
        let trimmed = trim_import_input(&path);
        if let Some(id) = package_id_from_path(&trimmed) {
            self.package_id = id;
        }
        self.path = trimmed;
        self.url.clear();
        self.message = None;
    }

    pub fn set_url(&mut self, url: String) {
        let trimmed = trim_import_input(&url);
        if let Some(id) = package_id_from_url(&trimmed) {
            self.package_id = id;
        }
        self.url = trimmed;
        self.path.clear();
        self.message = None;
    }

    pub fn set_clipboard_text(&mut self, text: String) {
        let trimmed = trim_import_input(&text);
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            self.set_url(trimmed);
        } else {
            self.set_path(trimmed);
        }
    }
}

#[derive(Clone, Default)]
pub struct MarketModelState {
    pub models: Vec<MarketModel>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub downloaded_ids: Vec<String>,
    pub downloading: Option<String>,
    pub download_progress: Option<f32>,
    pub install_message: Option<String>,
    pub install_message_since: Option<std::time::Instant>,
    /// 分类筛选（None=全部）。
    pub selected_tag: Option<String>,
    /// 每个模型选中的版本。
    pub selected_versions: HashMap<String, String>,
    /// 索引更新时间。
    pub updated_at: String,
}

#[derive(Clone, Default)]
pub struct MarketPluginState {
    pub plugins: Vec<MarketPlugin>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// 本地已安装插件（来自 xime-plugin registry）。
    pub installed: Vec<xime_plugin::PluginRecord>,
    pub downloaded_ids: Vec<String>,
    pub downloading: Option<String>,
    pub download_progress: Option<f32>,
    pub installing: Option<String>,
    pub install_message: Option<String>,
    pub install_message_since: Option<std::time::Instant>,
    /// 分类筛选（None=全部）。
    pub selected_tag: Option<String>,
    /// 索引更新时间。
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PluginIndex {
    pub index_version: u32,
    pub updated_at: String,
    #[serde(default)]
    pub plugins: Vec<MarketPlugin>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketPlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type", default)]
    pub plugin_type: String,
    #[serde(rename = "pluginType", default)]
    pub plugin_kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub homepage: Option<String>,
    #[serde(rename = "currentVersion")]
    pub current_version: Option<String>,
    #[serde(default)]
    pub versions: Vec<MarketPluginVersion>,
    #[serde(rename = "appVersion", default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketPluginVersion {
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(rename = "downloadUrl", default)]
    pub download_url: Vec<MarketDownloadUrl>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SchemaIndex {
    pub index_version: u32,
    pub updated_at: String,
    pub schemas: Vec<MarketSchema>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketSchema {
    pub id: String,
    pub name: String,
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub homepage: Option<String>,
    pub versions: Vec<MarketSchemaVersion>,
    #[serde(rename = "currentVersion")]
    pub current_version: Option<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketSchemaVersion {
    pub version: String,
    pub date: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(rename = "downloadUrl", default)]
    pub download_url: Vec<MarketDownloadUrl>,
    #[serde(default)]
    pub size: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketDownloadUrl {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(rename = "sizeBytes", default)]
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelIndex {
    pub index_version: u32,
    pub updated_at: String,
    #[serde(default)]
    pub models: Vec<MarketModel>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// prediction / handwriting / asr / other
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub size: String,
    #[serde(rename = "type")]
    #[serde(default)]
    pub model_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub homepage: Option<String>,
    #[serde(rename = "currentVersion")]
    pub current_version: Option<String>,
    #[serde(default)]
    pub versions: Vec<MarketModelVersion>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketModelVersion {
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub files: Vec<MarketModelFile>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MarketModelFile {
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(rename = "sizeBytes", default)]
    pub size_bytes: Option<u64>,
}

// ---- installation helpers ----

fn get_download_info(schema: &MarketSchema) -> Option<(&MarketDownloadUrl, String)> {
    let version = schema.current_version.as_deref().unwrap_or("latest");
    let info = schema
        .versions
        .iter()
        .find(|v| v.version == version || version == "latest")
        .or_else(|| schema.versions.first())?;
    let download = info.download_url.first()?;
    let ext = if download.url.ends_with(".zip") {
        ".zip"
    } else if download.url.ends_with(".tar.gz") {
        ".tar.gz"
    } else {
        return None;
    };
    Some((download, format!("{}{}", version, ext)))
}

/// 方案包清单管理器（rime 目录 = 用户数据目录；注册表在数据根 `.registry.yaml`）。
fn schema_manifest() -> Result<SchemaManifest, String> {
    let (_, user_data_dir) = get_data_dirs();
    let data_root = user_data_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| user_data_dir.clone());
    Ok(SchemaManifest::new(user_data_dir, data_root))
}

/// 注册表里非空的方案包 id（排序稳定，便于展示与比较）。
fn nonempty_packages(registry: &Registry) -> Vec<String> {
    let mut packages: Vec<String> = registry
        .iter()
        .filter(|(_, entry)| !entry.is_empty())
        .map(|(pkg, _)| pkg.clone())
        .collect();
    packages.sort();
    packages
}

/// 安装前冲突预检（对齐安卓 `installPackage` 的「rime 目录已有其他方案」判定）：
/// 先把无主方案文件登记为内置方案包，再取注册表里除自身外的全部方案包。
fn schema_install_conflict(schema_id: &str) -> Result<SchemaInstallConflict, String> {
    let manifest = schema_manifest()?;
    // 注册表已有内置包时跳过递归扫描（审计 P0：预检是安装点击后的头几百毫秒）。
    let mut registry = manifest.load_registry();
    if !registry.contains_key(BUILTIN_PACKAGE_ID) {
        manifest.refresh_builtin_package()?;
        registry = manifest.load_registry();
    }
    let packages: Vec<String> = nonempty_packages(&registry)
        .into_iter()
        .filter(|pkg| pkg != schema_id && pkg != LOCAL_PACKAGE_ID)
        .collect();
    Ok(SchemaInstallConflict {
        schema_id: schema_id.to_string(),
        packages,
    })
}

/// 还原内置方案前冲突预检：注册表里除内置方案包以外的全部方案包都要先卸载，
/// 否则还原后又会变成「内置 + 第三方」混装。
fn schema_restore_conflict() -> Result<Vec<String>, String> {
    let manifest = schema_manifest()?;
    let mut registry = manifest.load_registry();
    if !registry.contains_key(BUILTIN_PACKAGE_ID) {
        manifest.refresh_builtin_package()?;
        registry = manifest.load_registry();
    }
    Ok(nonempty_packages(&registry)
        .into_iter()
        .filter(|pkg| pkg != BUILTIN_PACKAGE_ID && pkg != LOCAL_PACKAGE_ID)
        .collect())
}

/// 从某方案包的文件清单（相对路径）里取出它**全部**的顶层方案 id：默认方案置顶，
/// 其余按字典序。子目录里的 `.schema.yaml` 不算（rime 的 schema_list 只认顶层 id）。
///
/// 安装/还原方案包时必须整包写进启用列表，而不是只写默认那一个：librime 只编译
/// `schema_list` 里的方案，只启用一个的话包内其余方案没有 build 产物，用户切过去
/// 会被服务端按「未部署」拒绝（见 xime-rime 的 `schema_deployed`）。
fn package_schema_ids<'a>(
    files: impl IntoIterator<Item = &'a str>,
    default_id: Option<&str>,
) -> Vec<String> {
    let mut ids: Vec<String> = files
        .into_iter()
        .filter_map(|file| file.strip_suffix(".schema.yaml"))
        .filter(|id| !id.contains('/'))
        .map(String::from)
        .collect();
    ids.sort();
    ids.dedup();
    if let Some(default) = default_id {
        if let Some(pos) = ids.iter().position(|id| id == default) {
            let default = ids.remove(pos);
            ids.insert(0, default);
        }
    }
    ids
}

/// 从还原出的内置方案文件里挑默认启用的方案：wubi86 优先（曜的默认方案），
/// 否则取字典序最靠前的顶层方案（子目录里的不算）。
fn restored_default_schema_id(files: &[String]) -> Option<String> {
    if files.iter().any(|f| f == "wubi86.schema.yaml") {
        return Some("wubi86".to_string());
    }
    files
        .iter()
        .filter_map(|f| f.strip_suffix(".schema.yaml"))
        .find(|id| !id.contains('/'))
        .map(String::from)
}

/// 还原内置方案后重写启用列表：先按默认方案置顶把内置包**全部**方案写进去
/// （只写默认那一个的话，包内其余方案没有 build 产物，切过去会被判「未部署」），
/// 顺带清掉可能残留的陈旧启用项（如第三方包卸载后仍指向 rime_ice 的条目）。
fn apply_restored_builtin_schema_list(manifest: &SchemaManifest) -> anyhow::Result<()> {
    let registry = manifest.load_registry();
    let Some(entry) = registry.get(BUILTIN_PACKAGE_ID) else {
        return Ok(());
    };
    let default_id = restored_default_schema_id(&entry.files);
    let ids = package_schema_ids(
        entry.files.iter().map(String::as_str),
        default_id.as_deref(),
    );
    if ids.is_empty() {
        return Ok(());
    }
    let manager = SchemaManager::new().map_err(|e| anyhow::anyhow!("{}", e))?;
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    manager
        .set_schema_list(&refs)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    manager.save().map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

/// 卸载方案包（对齐安卓 `uninstallWithManifest`）：无主文件先归内置方案包 →
/// 逐文件按 claimedBy 判断（共享文件保留，不误删内置/他人文件）→ 清理衍生产物 →
/// 启用列表移除该包名下全部方案 → 可选重新部署并通知宿主热载。
/// 用户主动卸载（`deploy = true`）且这是最后一个方案来源时，从备份自动还原
/// 内置方案包——「卸载第三方方案」即恢复默认方案。
///
/// `deploy = false` 供「先卸载冲突方案包再安装/还原」流程使用（安装流程统一部署一次，
/// 避免多次数秒级部署）。
fn do_uninstall(schema_id: &str, deploy: bool) -> anyhow::Result<()> {
    let manifest = schema_manifest().map_err(|e| anyhow::anyhow!(e))?;
    // 无主文件先归入内置方案包，避免卸载时漏删/误删
    // （对齐安卓 confirmInstallWithUninstall 里的 refreshBuiltinManifest）。
    manifest
        .refresh_builtin_package()
        .map_err(|e| anyhow::anyhow!(e))?;
    let outcome = manifest
        .uninstall_package(schema_id)
        .map_err(|e| anyhow::anyhow!(e))?;

    let manager = SchemaManager::new().map_err(|e| anyhow::anyhow!("{}", e))?;
    // 包 id ≠ 方案 id（rime-ice → rime_ice）：注册表知道该包名下的全部方案 id。
    let mut removed: Vec<String> = outcome.removed_schema_ids.clone();
    removed.push(schema_id.to_string());
    let mut remaining: Vec<String> = manager
        .get_schema_list_ids()
        .into_iter()
        .filter(|id| !removed.iter().any(|r| r == id))
        .collect();
    if remaining.is_empty() && deploy {
        // 启用列表清空 = 卸掉了最后一个方案来源：从备份还原内置方案包。
        // 注册表里还有其他市场包时不还原（避免混装——它们的方案只是未启用）；
        // 没有备份时走下方回退。
        let other_market_packages = nonempty_packages(&manifest.load_registry())
            .into_iter()
            .any(|pkg| pkg != BUILTIN_PACKAGE_ID && pkg != LOCAL_PACKAGE_ID);
        if !manifest.is_builtin_removed()
            && !other_market_packages
            && manifest.restore_builtin_package().is_ok()
        {
            let files = manifest
                .load_registry()
                .get(BUILTIN_PACKAGE_ID)
                .map(|e| e.files.clone())
                .unwrap_or_default();
            // 整包启用（默认方案置顶），与安装/还原语义一致：只启用一个的话
            // 内置包内其余方案没有 build 产物，切换时会被判「未部署」。
            let default_id = restored_default_schema_id(&files);
            remaining = package_schema_ids(files.iter().map(String::as_str), default_id.as_deref());
        }
    }
    if remaining.is_empty() {
        // 启用列表清空：回退到仍存在的方案，避免 rime 因 schema_list 为空异常。
        remaining = manager
            .get_schema_list()
            .into_iter()
            .map(|s| s.schema_id)
            .filter(|id| !removed.iter().any(|r| r == id))
            .collect();
    }
    if !remaining.is_empty() {
        let refs: Vec<&str> = remaining.iter().map(String::as_str).collect();
        manager
            .set_schema_list(&refs)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }
    manager.save().map_err(|e| anyhow::anyhow!("{}", e))?;

    if deploy {
        deploy_all().map_err(|e| anyhow::anyhow!("部署失败: {}", e))?;
        notify_daemon_reload();
    }
    Ok(())
}

fn do_download(schema: &MarketSchema) -> anyhow::Result<()> {
    let (download, filename) =
        get_download_info(schema).ok_or_else(|| anyhow::anyhow!("无可用下载地址或不支持的格式"))?;

    let dest_dir = market_dir().join(&schema.id);
    std::fs::create_dir_all(&dest_dir)?;

    let dest = dest_dir.join(&filename);
    download_file(
        &download.url,
        &dest,
        download.sha256.as_deref(),
        |progress| {
            *download_progress().lock().unwrap() = Some((schema.id.clone(), progress as f32));
        },
    )?;
    Ok(())
}

fn do_download_model(model: &MarketModel) -> anyhow::Result<()> {
    let version = model
        .versions
        .iter()
        .find(|v| Some(v.version.as_str()) == model.current_version.as_deref())
        .or_else(|| model.versions.first())
        .ok_or_else(|| anyhow::anyhow!("无可用版本"))?;
    anyhow::ensure!(
        !version.files.is_empty(),
        "无可用下载文件（version: {}）",
        version.version
    );

    let dest_dir = models_dir().join(&model.id);
    std::fs::create_dir_all(&dest_dir)?;

    let total_bytes: u64 = version
        .files
        .iter()
        .map(|f| f.size_bytes.unwrap_or(0))
        .sum();

    let mut accumulated: u64 = 0;
    for file in &version.files {
        anyhow::ensure!(!file.url.is_empty(), "缺少下载地址（{}）", file.name);
        let dest = dest_dir.join(&file.name);
        let file_bytes = file.size_bytes.unwrap_or(0);
        let sha256 = file.sha256.as_deref();
        download_file(&file.url, &dest, sha256, |progress| {
            let overall = accumulated as f64 / total_bytes.max(1) as f64
                + progress * file_bytes as f64 / total_bytes.max(1) as f64;
            *download_progress().lock().unwrap() = Some((model.id.clone(), overall as f32));
        })?;
        accumulated += file_bytes;
    }
    Ok(())
}

/// 下载插件 .xipk 到插件目录（plugins/<id>.xipk，与已安装插件 plugins/<id>/ 目录同根）。
/// 下载插件包到临时文件并返回路径（调用方负责安装后删除）。
fn do_download_plugin(plugin: &MarketPlugin) -> anyhow::Result<std::path::PathBuf> {
    let version = plugin
        .versions
        .iter()
        .find(|v| Some(v.version.as_str()) == plugin.current_version.as_deref())
        .or_else(|| plugin.versions.first())
        .ok_or_else(|| anyhow::anyhow!("无可用版本"))?;
    let download = version
        .download_url
        .first()
        .ok_or_else(|| anyhow::anyhow!("缺少下载地址"))?;
    anyhow::ensure!(!download.url.is_empty(), "缺少下载地址");

    let dest = std::env::temp_dir().join(format!("{}.xipk", plugin.id));
    download_file(
        &download.url,
        &dest,
        download.sha256.as_deref(),
        |progress| {
            *download_progress().lock().unwrap() = Some((plugin.id.clone(), progress as f32));
        },
    )?;
    Ok(dest)
}

/// 从插件目录安装已下载的插件包。
fn do_install_plugin(plugin_id: &str) -> anyhow::Result<()> {
    let xipk = plugins_dir().join(format!("{plugin_id}.xipk"));
    anyhow::ensure!(xipk.exists(), "未找到下载的插件包，请先下载");

    let manager = plugin_manager();
    manager
        .install_from_zip(&xipk, true)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    std::fs::remove_file(&xipk).ok();
    Ok(())
}

/// 下载单个文件到 dest，可选 sha256 校验；进度经回调上报（0.0~1.0）。
fn download_file(
    url: &str,
    dest: &std::path::Path,
    sha256: Option<&str>,
    on_progress: impl Fn(f64),
) -> anyhow::Result<()> {
    download_file_with_stats(url, dest, sha256, |done, total, _bps| {
        if total > 0 {
            on_progress(done as f64 / total as f64);
        }
    })
}

/// 下载并回调 `(已下载字节, 总字节[0=未知], 字节/秒)`，供导入进度条使用。
fn download_file_with_stats(
    url: &str,
    dest: &std::path::Path,
    sha256: Option<&str>,
    on_progress: impl Fn(u64, u64, u64),
) -> anyhow::Result<()> {
    let mut response = http_client()
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow::anyhow!("网络请求失败: {}", e))?;
    let total = response.content_length().unwrap_or(0);

    let mut hasher = sha256.map(|_| sha2::Sha256::new());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("创建目录失败: {}", e))?;
    }
    // 先写 .part 临时文件、全部成功后再 rename：直写最终路径的话，任何
    // 中途失败都会留残缺文件在"已下载"位置（目录存在即被判为已下载，
    // 之后安装永远失败且用户无从自愈）。
    let tmp = dest.with_extension(format!(
        "{}part.{}",
        dest.extension()
            .map(|e| format!("{}.part", e.to_string_lossy()))
            .unwrap_or_else(|| "part".to_string()),
        std::process::id()
    ));
    let mut output =
        std::fs::File::create(&tmp).map_err(|e| anyhow::anyhow!("创建文件失败: {}", e))?;

    let result = (|| -> anyhow::Result<()> {
        let mut buf = [0u8; 8192];
        let mut done: u64 = 0;
        let started = std::time::Instant::now();
        let mut last_report = started;
        loop {
            let n = response
                .read(&mut buf)
                .map_err(|e| anyhow::anyhow!("读取响应失败: {}", e))?;
            if n == 0 {
                break;
            }
            if let Some(h) = &mut hasher {
                h.update(&buf[..n]);
            }
            output
                .write_all(&buf[..n])
                .map_err(|e| anyhow::anyhow!("写入文件失败: {}", e))?;
            done += n as u64;
            let now = std::time::Instant::now();
            if now.duration_since(last_report).as_millis() >= 200 {
                let elapsed = now.duration_since(started).as_secs_f64().max(0.001);
                let bps = (done as f64 / elapsed) as u64;
                on_progress(done, total, bps);
                last_report = now;
            }
        }
        on_progress(done, total, 0);

        if let (Some(h), Some(expected)) = (hasher, sha256) {
            let actual = hex::encode(h.finalize());
            if !actual.eq_ignore_ascii_case(expected.trim()) {
                anyhow::bail!("文件校验失败（sha256 不匹配），文件可能不完整");
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            drop(output);
            std::fs::rename(&tmp, dest).map_err(|e| anyhow::anyhow!("落盘失败: {}", e))?;
            Ok(())
        }
        Err(e) => {
            drop(output);
            std::fs::remove_file(&tmp).ok();
            Err(e)
        }
    }
}

/// 常见方案包会多包一层同名目录（`pkg/pkg/*.schema.yaml`）；只有一层目录且
/// 根下没有文件时，把那一层当归档根，避免安装时找不到顶层 `.schema.yaml`。
fn unwrap_single_archive_dir(root: &std::path::Path) -> std::path::PathBuf {
    let mut current = root.to_path_buf();
    loop {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return current;
        };
        let mut dirs = Vec::new();
        let mut has_file = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if !entry.file_name().to_string_lossy().starts_with('.') {
                has_file = true;
            }
        }
        if has_file || dirs.len() != 1 {
            if has_top_schema(&current) {
                return current;
            }
            // 根下可能混了 README/__MACOSX 等多个目录：找第一个带顶层 schema 的子目录。
            for dir in &dirs {
                if has_top_schema(dir) {
                    return dir.clone();
                }
            }
            if let Some(found) = find_schema_dir(&dirs) {
                return found;
            }
            return current;
        }
        current = dirs.remove(0);
    }
}

fn has_top_schema(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry.file_name().to_string_lossy().ends_with(".schema.yaml")
            })
        })
        .unwrap_or(false)
}

fn find_schema_dir(dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    for dir in dirs {
        if has_top_schema(dir) {
            return Some(dir.clone());
        }
        let children: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(found) = find_schema_dir(&children) {
            return Some(found);
        }
    }
    None
}

/// 递归收集解压目录下全部文件（相对路径统一 `/` 分隔，对齐归档条目路径）。
fn collect_release_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_release_files(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}

/// 安装前预检：解压到临时目录，确认能找到顶层 `.schema.yaml`。
/// 必须在卸载冲突包**之前**调用，避免坏包把现有方案删光。
fn validate_package_archive(schema_id: &str) -> anyhow::Result<()> {
    let cache_dir = market_dir().join(schema_id);
    anyhow::ensure!(cache_dir.exists(), "未找到缓存的下载文件");
    let archive = std::fs::read_dir(&cache_dir)?
        .flatten()
        .find(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.ends_with(".zip") || n.ends_with(".tar.gz")
        })
        .ok_or_else(|| anyhow::anyhow!("未找到缓存的压缩包"))?
        .path();

    let temp_dir = std::env::temp_dir().join(format!(
        "xime_validate_{}_{}",
        std::process::id(),
        schema_id
    ));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    std::fs::create_dir_all(&temp_dir)?;

    let result = (|| -> anyhow::Result<()> {
        let name = archive.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".zip") {
            extract_zip(&archive, &temp_dir)?;
        } else {
            extract_tar_gz(&archive, &temp_dir)?;
        }
        let root = unwrap_single_archive_dir(&temp_dir);
        let mut files = Vec::new();
        collect_release_files(&root, &root, &mut files)?;
        let has_schema = files
            .iter()
            .any(|(rel, _)| rel.ends_with(".schema.yaml") && !rel.contains('/'));
        anyhow::ensure!(
            has_schema,
            "包内没有顶层 .schema.yaml，不是可安装的方案包（常见原因：压缩包多套了一层目录）"
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&temp_dir).ok();
    result
}

/// 从 market 包安装方案（对齐 Android installPackageFromMarketDir）：
/// 全量释放（不只 .schema.yaml，含词典/lua 等依赖文件）→ 冲突检测
/// （目标文件已被其他包/内置方案包占用且内容不同则拒绝）→ 写包清单
/// （按包隔离，含内容哈希，卸载据此精确删文件、按 claimedBy 保护共享文件）。
fn do_install(schema_id: &str) -> anyhow::Result<()> {
    let cache_schema_dir = market_dir().join(schema_id);
    anyhow::ensure!(cache_schema_dir.exists(), "未找到缓存的下载文件");

    let archive = std::fs::read_dir(&cache_schema_dir)?
        .flatten()
        .find(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.ends_with(".zip") || n.ends_with(".tar.gz")
        })
        .ok_or_else(|| anyhow::anyhow!("未找到缓存的压缩包"))?;

    let archive_path = archive.path();
    let temp_dir = std::env::temp_dir().join(format!("xime_extract_{}", schema_id));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    std::fs::create_dir_all(&temp_dir)?;

    let filename = archive_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let extract_result = if filename.ends_with(".zip") {
        extract_zip(&archive_path, &temp_dir)
    } else {
        extract_tar_gz(&archive_path, &temp_dir)
    };
    if let Err(e) = extract_result {
        // 损坏的包直接丢弃（对齐 Android validateArchive 失败删除）。
        std::fs::remove_dir_all(&temp_dir).ok();
        return Err(e);
    }

    let (_, user_data_dir) = get_data_dirs();
    let manifest = schema_manifest().map_err(|e| anyhow::anyhow!(e))?;
    // 无主方案文件先登记为内置方案包（对齐安卓「每次安装前 refreshBuiltinManifest」）：
    // 第三方包不得悄悄把内置方案文件收进自己的清单。
    manifest
        .refresh_builtin_package()
        .map_err(|e| anyhow::anyhow!(e))?;

    // 1) 全量释放清单：归档内容进 rime 目录，但过滤宿主/引擎自有文件
    //    （default.yaml / xime.yaml / custom_phrase.txt 等）、部署产物（build/）、
    //    用户词典目录（*.userdb/）与其他平台前端配置。
    let mut release_files: Vec<(String, std::path::PathBuf)> = Vec::new();
    let release_root = unwrap_single_archive_dir(&temp_dir);
    if let Err(e) = collect_release_files(&release_root, &release_root, &mut release_files) {
        std::fs::remove_dir_all(&temp_dir).ok();
        return Err(e);
    }
    release_files.retain(|(rel, _)| !schema_manifest::is_protected_release_path(rel));
    anyhow::ensure!(!release_files.is_empty(), "下载包中没有可安装的文件");

    // 发现真实方案 id（包 id ≠ 方案 id，如 rime-ice → rime_ice；从顶层
    // *.schema.yaml 提取），并优先取与包 id 规范化后同名的为启用目标。
    let mut new_schema_ids: Vec<String> = release_files
        .iter()
        .filter_map(|(rel, _)| rel.strip_suffix(".schema.yaml"))
        .filter(|rel| !rel.contains('/'))
        .map(|rel| rel.to_string())
        .collect();
    new_schema_ids.sort();
    new_schema_ids.dedup();
    anyhow::ensure!(
        !new_schema_ids.is_empty(),
        "包内没有 .schema.yaml，不是可安装的方案包"
    );
    let normalized = schema_id.replace('-', "_");
    let enabled_id = new_schema_ids
        .iter()
        .find(|id| *id == &normalized)
        .or_else(|| new_schema_ids.first())
        .cloned()
        .unwrap_or_else(|| schema_id.to_string());

    // 2) 冲突检测（对齐安卓 detectConflicts）：目标文件已被其他包（含内置方案包）
    //    声明且内容不同 → 拒绝安装；同内容视为共享依赖放行；同包重装/升级放行。
    let mut targets: Vec<(String, String)> = Vec::with_capacity(release_files.len());
    for (rel, src) in &release_files {
        let hash = schema_manifest::sha256_file(src)
            .ok_or_else(|| anyhow::anyhow!("读取待安装文件失败：{}", rel))?;
        targets.push((rel.clone(), hash));
    }
    let conflicts = manifest.detect_conflicts(schema_id, &targets);
    if !conflicts.is_empty() {
        std::fs::remove_dir_all(&temp_dir).ok();
        let detail = conflicts
            .iter()
            .map(|c| c.describe())
            .collect::<Vec<_>>()
            .join("；");
        anyhow::bail!(
            "文件冲突：{detail}。本机同一时刻只保留一个方案包，\
             请先在「已安装」里卸载占用方（或直接用安装确认弹窗卸载后安装）"
        );
    }

    // 3) 释放到 rime 目录（保留相对路径结构）。
    for (rel, src) in &release_files {
        let dest = user_data_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, &dest)?;
    }
    std::fs::remove_dir_all(&temp_dir).ok();

    // 4) 写安装清单（按包隔离，含内容哈希：卸载据此精确删文件、按 claimedBy
    //    保护共享文件）。
    manifest
        .register_package(schema_id, &targets)
        .map_err(|e| anyhow::anyhow!(e))?;

    // 5) 启用该包的全部方案（默认方案置顶；列表替换而非追加，避免包 id ≠
    //    方案 id 时塞入 rime 不认识的条目）。
    //
    //    必须整包启用：librime 只编译 schema_list 里的方案，只启用一个的话
    //    包内其余方案没有 build 产物，用户切过去会被判「未部署」而拒绝。
    let enabled_ids = package_schema_ids(
        release_files.iter().map(|(rel, _)| rel.as_str()),
        Some(&enabled_id),
    );
    anyhow::ensure!(!enabled_ids.is_empty(), "包内没有可启用的方案");
    let refs: Vec<&str> = enabled_ids.iter().map(String::as_str).collect();
    let manager = SchemaManager::new().map_err(|e| anyhow::anyhow!("{}", e))?;
    manager
        .set_schema_list(&refs)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    manager.save().map_err(|e| anyhow::anyhow!("{}", e))?;

    deploy_all().map_err(|e| anyhow::anyhow!("部署失败: {}", e))?;
    notify_daemon_reload();

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

fn extract_zip(archive_path: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let target = dest.join(&path);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut output = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut output)?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive_path: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let target = dest.join(&path);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_schema_ids_ignore_empty_placeholder_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "xime_downloaded_ids_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cangjie")).unwrap();
        std::fs::create_dir_all(dir.join("quick")).unwrap();
        std::fs::create_dir_all(dir.join("rime-ice")).unwrap();
        std::fs::write(dir.join("rime-ice").join("rime-ice.zip"), "zip").unwrap();
        let ids = scan_downloaded_schema_ids(&dir);
        assert_eq!(ids, vec!["rime-ice".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_package_id_derives_and_sanitizes() {
        assert_eq!(
            sanitize_package_id("rime-ice.zip").as_deref(),
            Some("rime-ice")
        );
        assert_eq!(
            sanitize_package_id("wubi86.tar.gz").as_deref(),
            Some("wubi86")
        );
        assert_eq!(
            sanitize_package_id("my scheme!.zip").as_deref(),
            Some("my scheme")
        );
        assert_eq!(sanitize_package_id("../evil.zip").as_deref(), Some("evil"));
        assert_eq!(sanitize_package_id("..."), None);
        assert_eq!(
            package_id_from_path("C:\\x\\rime-ice.zip").as_deref(),
            Some("Rime Ice")
        );
        assert_eq!(
            package_id_from_url("https://example.com/a/rime-ice.zip?x=1").as_deref(),
            Some("Rime Ice")
        );
        assert_eq!(package_id_from_url("https://example.com/a/readme.txt"), None);
    }

    #[test]
    fn import_only_accepts_archives() {
        assert!(archive_name_ok("a.zip"));
        assert!(archive_name_ok("a.tar.gz"));
        assert!(!archive_name_ok("a.rar"));
        assert!(!archive_name_ok("a.zip.tmp"));
    }

    #[test]
    fn archive_single_top_dir_is_unwrapped() {
        let base = std::env::temp_dir().join(format!("xime_archive_root_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let nested = base.join("rime-ice").join("rime-ice");
        std::fs::create_dir_all(&nested).ok();
        std::fs::write(nested.join("rime_ice.schema.yaml"), "schema").ok();
        let root = unwrap_single_archive_dir(&base);
        assert_eq!(root, nested);
        std::fs::remove_dir_all(&base).ok();
    }

    /// 手动跑（`--ignored --nocapture`）：复现商店「获取索引失败」的真实原因。
    ///
    /// 走的就是设置页同一条路径（`http_client()` + 同一个 URL），所以打印出来的
    /// 错误链与用户看到的一致；不联网时不要跑。
    #[test]
    #[ignore = "需要联网，手动跑"]
    fn rimes_index_reachable() {
        let url = "https://index.ximei.me/rimes/index.yaml";
        match http_client().get(url).send() {
            Ok(response) => {
                let status = response.status();
                match response.text() {
                    Ok(text) => println!("OK status={status} bytes={}", text.len()),
                    Err(e) => println!("READ FAIL: {e:?}"),
                }
            }
            Err(e) => {
                println!("SEND FAIL: {e:?}");
                let mut source = std::error::Error::source(&e);
                while let Some(err) = source {
                    println!("  caused by: {err}");
                    source = err.source();
                }
            }
        }
    }

    /// v2 插件子索引（JS/QuickJS 插件）的裁剪样本——字段形态取自
    /// https://index.ximei.me/plugins/v2/index.yaml。
    const PLUGIN_INDEX_V2_YAML: &str = r#"
index_version: 2
updated_at: '2026-09-26'
plugins:
- id: com.kingzcheung.xime.plugin.ai_reply
  name: AI 智能回复
  author: Xime
  description: 根据对方消息生成回复候选。
  type: remote
  tags:
  - AI
  pluginType: tool
  icon: 应
  activation: multi
  minHostVersion: 3.0.0
  platforms:
  - android
  capabilities:
    tool:
      display: passive
  network:
    allowCustomHosts: true
  homepage: https://example.com
  license: GPL-3.0
  appVersion: '>=3.0.0'
  currentVersion: 1.0.0
  versions:
  - version: 1.0.0
    date: '2026-09-24'
    changelog: v3 重构：TypeScript/QuickJS 插件
    downloadUrl:
    - url: https://example.com/ai-reply-1.0.0.xipk
      sha256: 4e5454210000000000000000000000000000000000000000000000000000000
      size: 3.4 KB
      sizeBytes: 3469
"#;

    #[test]
    fn parse_plugin_index_v2() {
        let index: PluginIndex = serde_yaml::from_str(PLUGIN_INDEX_V2_YAML).unwrap();
        assert_eq!(index.index_version, 2);
        assert_eq!(index.updated_at, "2026-09-26");
        assert_eq!(index.plugins.len(), 1);

        let plugin = &index.plugins[0];
        assert_eq!(plugin.id, "com.kingzcheung.xime.plugin.ai_reply");
        assert_eq!(plugin.plugin_kind, "tool", "pluginType 字段应被解析");
        assert_eq!(plugin.current_version.as_deref(), Some("1.0.0"));

        let version = &plugin.versions[0];
        assert_eq!(version.version, "1.0.0");
        assert_eq!(version.download_url.len(), 1);
        assert_eq!(
            version.download_url[0].url,
            "https://example.com/ai-reply-1.0.0.xipk"
        );
        assert!(
            version.download_url[0].sha256.is_some(),
            "sha256 用于下载校验"
        );
    }

    const MODEL_INDEX_YAML: &str = r#"
index_version: 1
updated_at: '2026-08-12'
models:
- id: predictive-text-small
  name: 智能联想模型 small 版本
  author: kingzcheung
  description: 基于 ONNX 的 AI 联想词预测模型
  category: prediction
  size: 18.9 MB
  type: remote
  tags: [联想, AI, ONNX]
  homepage: https://github.com/ximeiorg/predictive-text
  currentVersion: v1.0
  versions:
  - version: v1.0
    date: '2026-08-01'
    changelog: 初始版本
    files:
    - name: vocab.json
      url: https://example.com/vocab.json
      sha256: 3f7a6aa773afe6dacf75701f7861257d36a46a26ac70c0f8ee6dd4032cc3b9c2
      size: 137.8 KB
      sizeBytes: 141139
    - name: model_int8_dynamic.onnx
      url: https://example.com/model.onnx
      sha256: e15009c84d9702056ba8b5f6c04b27ae7d0400167647a5e94cb699f24f885a9d
      size: 34.7 MB
      sizeBytes: 36353598
- id: ochwpro
  name: 手写模型
  category: handwriting
  size: 6.7 MB
  currentVersion: v1.0
  versions:
  - version: v1.0
    files:
    - name: ochwpro.onnx
      url: https://example.com/ochwpro.onnx
"#;

    #[test]
    fn parse_model_index() {
        let index: ModelIndex = serde_yaml::from_str(MODEL_INDEX_YAML).unwrap();
        assert_eq!(index.updated_at, "2026-08-12");
        assert_eq!(index.models.len(), 2);

        let model = &index.models[0];
        assert_eq!(model.id, "predictive-text-small");
        assert_eq!(model.category, "prediction");
        assert_eq!(model.current_version.as_deref(), Some("v1.0"));
        assert_eq!(model.versions[0].files.len(), 2);
        let file = &model.versions[0].files[0];
        assert_eq!(file.name, "vocab.json");
        assert_eq!(file.size_bytes, Some(141139));
        assert_eq!(
            file.sha256.as_deref(),
            Some("3f7a6aa773afe6dacf75701f7861257d36a46a26ac70c0f8ee6dd4032cc3b9c2")
        );

        let handwriting = &index.models[1];
        assert_eq!(handwriting.category, "handwriting");
        assert_eq!(handwriting.author, "");
        assert!(handwriting.versions[0].files[0].sha256.is_none());
    }

    const SCHEMA_INDEX_YAML: &str = r#"
index_version: 1
updated_at: '2026-08-12'
schemas:
- id: rime-frost
  name: 白霜拼音
  author: gaboolic
  description: 白霜拼音
  type: remote
  tags: [拼音, 双拼]
  dependencies: [luna_pinyin]
  currentVersion: 1.0.4
  versions:
  - version: 1.0.4
    date: '2026-07-10'
    downloadUrl:
    - url: https://github.com/gaboolic/rime-frost/releases/download/1.0.4/rime-frost-schemas.zip
      sha256: 4f4998ae83f63d757c0a4ace192f69d48265bddfabe231642b73e3739ed0f2f5
      size: 42 MB
"#;

    #[test]
    fn parse_schema_index_and_pick_download() {
        let index: SchemaIndex = serde_yaml::from_str(SCHEMA_INDEX_YAML).unwrap();
        assert_eq!(index.schemas.len(), 1);

        let schema = &index.schemas[0];
        assert_eq!(schema.schema_type, "remote");
        assert_eq!(schema.tags, vec!["拼音".to_string(), "双拼".to_string()]);
        assert_eq!(schema.current_version.as_deref(), Some("1.0.4"));

        let (download, filename) = get_download_info(schema).unwrap();
        assert_eq!(
            download.url,
            "https://github.com/gaboolic/rime-frost/releases/download/1.0.4/rime-frost-schemas.zip"
        );
        assert_eq!(
            download.sha256.as_deref(),
            Some("4f4998ae83f63d757c0a4ace192f69d48265bddfabe231642b73e3739ed0f2f5")
        );
        assert_eq!(filename, "1.0.4.zip");
    }

    /// 整包启用：包内**全部**顶层方案都要进启用列表，默认方案置顶。
    /// 只写一个的话包内其余方案没有 build 产物，切换会被服务端判「未部署」。
    #[test]
    fn package_schema_ids_keeps_whole_package_with_default_first() {
        let files = [
            "wubi86.dict.yaml".to_string(),
            "wubi86.schema.yaml".to_string(),
            "wubi86_pinyin.schema.yaml".to_string(),
            "wubi86_trad.schema.yaml".to_string(),
            "pinyin_simp.schema.yaml".to_string(),
            "symbols.yaml".to_string(),
        ];
        let ids = package_schema_ids(files.iter().map(String::as_str), Some("wubi86"));
        assert_eq!(
            ids,
            vec![
                "wubi86".to_string(),
                "pinyin_simp".to_string(),
                "wubi86_pinyin".to_string(),
                "wubi86_trad".to_string(),
            ]
        );
    }

    /// 默认方案不在包里（或没给）时按字典序，不塞入不存在的方案。
    #[test]
    fn package_schema_ids_default_missing_falls_back_to_sorted() {
        let files = ["b.schema.yaml".to_string(), "a.schema.yaml".to_string()];
        let ids = package_schema_ids(files.iter().map(String::as_str), Some("nope"));
        assert_eq!(ids, ["a".to_string(), "b".to_string()]);
        let ids = package_schema_ids(files.iter().map(String::as_str), None);
        assert_eq!(ids, ["a".to_string(), "b".to_string()]);
    }

    /// 子目录里的 `.schema.yaml` 不算（rime 的 schema_list 只认顶层 id），重复项去重。
    #[test]
    fn package_schema_ids_ignores_nested_and_dedups() {
        let files = [
            "nested/inner.schema.yaml".to_string(),
            "top.schema.yaml".to_string(),
            "top.schema.yaml".to_string(),
            "lua/uuid.lua".to_string(),
        ];
        let ids = package_schema_ids(files.iter().map(String::as_str), Some("top"));
        assert_eq!(ids, vec!["top".to_string()]);
    }

    /// `build/` 不存在时视为「没有任何方案已部署」，不 panic。
    #[test]
    fn deployed_schema_ids_in_missing_dir_is_empty() {
        let dir = std::env::temp_dir().join("xime_no_such_build_dir_4242");
        assert!(deployed_schema_ids_in(&dir).is_empty());
    }

    #[test]
    fn scan_dir_ids_skips_hidden_and_files() {
        let dir = std::env::temp_dir().join(format!("xime_scan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("model-a")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::create_dir_all(dir.join("model-b")).unwrap();
        std::fs::write(dir.join("plain-file"), b"x").unwrap();

        let mut ids = scan_dir_ids(&dir);
        ids.sort();
        assert_eq!(ids, vec!["model-a".to_string(), "model-b".to_string()]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn format_size_helper() {
        assert_eq!(crate::pages::store::format_size("42 MB"), "42 mb");
        assert_eq!(crate::pages::store::format_size("7001270"), "6.7 MB");
        assert_eq!(crate::pages::store::format_size("1024"), "1024 B");
        assert_eq!(crate::pages::store::format_size("2048"), "2.0 KB");
        assert_eq!(crate::pages::store::format_size("500"), "500 B");
    }

    /// 列表翻页：总页数、末页不前进、越界夹回、条目缩减后夹回。
    /// （不触库：直接构造状态，历史与快捷发送共用同一套页逻辑）
    #[cfg(feature = "clipboard-page")]
    #[test]
    fn list_pagination_pages_and_clamps() {
        let mk = |n: usize| ClipboardHistoryState {
            items: (0..n)
                .map(|i| xime_config::clipboard_store::ClipboardHistoryItem {
                    id: i as i64,
                    text: format!("t{i}"),
                    timestamp: 0,
                })
                .collect(),
            selected: None,
            page: 0,
        };

        // 17 条 → 3 页
        let mut h = mk(17);
        assert_eq!(h.total_pages(), 3);
        h.next_page();
        h.next_page();
        h.next_page(); // 已在末页，不再前进
        assert_eq!(h.page, 2);
        h.prev_page();
        assert_eq!(h.page, 1);
        // 空列表至少 1 页
        assert_eq!(mk(0).total_pages(), 1);

        // 条目缩减后页码夹回有效范围
        h.page = 9;
        h.clamp_page();
        assert_eq!(h.page, 2);
        h.items.truncate(8);
        h.clamp_page();
        assert_eq!(h.page, 0);
    }

    #[test]
    fn restored_default_schema_prefers_wubi86() {
        let files = vec![
            "build/wubi98.table.bin".to_string(),
            "pinyin_simp.schema.yaml".to_string(),
            "symbols.yaml".to_string(),
            "wubi86.schema.yaml".to_string(),
        ];
        assert_eq!(
            restored_default_schema_id(&files).as_deref(),
            Some("wubi86")
        );

        // 没有 wubi86 时取字典序最靠前的顶层方案；子目录里的不算、非方案文件不算。
        let files = vec![
            "sub/nested.schema.yaml".to_string(),
            "symbols.yaml".to_string(),
            "wubi98.schema.yaml".to_string(),
        ];
        assert_eq!(
            restored_default_schema_id(&files).as_deref(),
            Some("wubi98")
        );
        assert_eq!(
            restored_default_schema_id(&["symbols.yaml".to_string()]),
            None
        );
    }
}

/// 用户词典「浏览词条」子视图的纯逻辑（分页/防抖/截断提示）。
#[cfg(all(test, windows))]
mod dict_browse_tests {
    use super::*;

    fn rows(count: usize) -> Vec<DictEntryRow> {
        (0..count)
            .map(|i| DictEntryRow {
                word: format!("词{i}"),
                code: format!("code{i}"),
                commits: 1,
            })
            .collect()
    }

    fn browse_with(entries: Vec<DictEntryRow>, page: usize) -> DictBrowseState {
        let matched = entries.len() as i32;
        DictBrowseState {
            dict: "wubi86".to_string(),
            entries,
            matched,
            page,
            loaded: true,
            ..DictBrowseState::default()
        }
    }

    #[test]
    fn pages_split_into_full_and_partial_slices() {
        let browse = browse_with(rows(DICT_ENTRIES_PAGE_SIZE * 2 + 20), 0);
        assert_eq!(browse.page_count(), 3);

        assert_eq!(browse.page_entries().len(), DICT_ENTRIES_PAGE_SIZE);
        assert_eq!(browse.page_entries()[0].word, "词0");

        let last = browse_with(rows(DICT_ENTRIES_PAGE_SIZE * 2 + 20), 2);
        assert_eq!(last.page_entries().len(), 20);
        assert_eq!(
            last.page_entries()[0].word,
            format!("词{}", DICT_ENTRIES_PAGE_SIZE * 2)
        );

        // 越界页给空切片，不 panic（view 层不会崩）。
        let beyond = browse_with(rows(10), 5);
        assert!(beyond.page_entries().is_empty());
    }

    #[test]
    fn empty_and_exact_page_counts() {
        assert_eq!(browse_with(Vec::new(), 0).page_count(), 0);
        assert_eq!(browse_with(rows(1), 0).page_count(), 1);
        assert_eq!(browse_with(rows(DICT_ENTRIES_PAGE_SIZE), 0).page_count(), 1);
        assert_eq!(
            browse_with(rows(DICT_ENTRIES_PAGE_SIZE + 1), 0).page_count(),
            2
        );
    }

    #[test]
    fn truncated_only_when_returned_entries_fall_short_of_hits() {
        let mut browse = browse_with(rows(DICT_ENTRIES_MAX), 0);
        assert!(!browse.truncated(), "回传条数与命中数相同就不算截断");

        browse.matched = DICT_ENTRIES_MAX as i32 + 40;
        assert!(browse.truncated(), "命中多于回传条数才算截断");
    }

    #[test]
    fn query_change_resets_page_and_arms_debounce() {
        let mut dict = DictManageState {
            browse: browse_with(rows(120), 2),
            ..DictManageState::default()
        };

        dict.browse_set_query("gou".to_string());
        assert_eq!(dict.browse.query, "gou");
        assert_eq!(dict.browse.page, 0, "换了关键词要回到第一页");
        assert!(dict.browse.pending.is_some(), "关键词变化应触发防抖读取");

        // 同样的关键词（iced 每帧都会回报当前值）不重复排队。
        dict.browse.pending = None;
        dict.browse_set_query("gou".to_string());
        assert!(dict.browse.pending.is_none());
    }

    #[test]
    fn page_navigation_clamps_to_range() {
        let mut dict = DictManageState {
            browse: browse_with(rows(120), 0),
            ..DictManageState::default()
        };
        dict.browse_page(2);
        assert_eq!(dict.browse.page, 2);
        dict.browse_page(99);
        assert_eq!(dict.browse.page, 2, "越界夹到末页");

        // 没有词条时永远停在第 0 页。
        dict.browse = browse_with(Vec::new(), 3);
        dict.browse_page(1);
        assert_eq!(dict.browse.page, 0);
    }

    /// 下拉切换词典：清关键词/翻页/对话框/确认态并立刻发首读。
    #[test]
    fn browse_select_resets_state_and_fetches_new_dict() {
        let mut dict = DictManageState {
            browse: DictBrowseState {
                dict: "wubi86".to_string(),
                query: "gou".to_string(),
                entries: rows(5),
                page: 2,
                loaded: true,
                ..DictBrowseState::default()
            },
            ..DictManageState::default()
        };

        dict.browse_select("rime_ice".to_string());
        assert_eq!(dict.browse.dict, "rime_ice");
        assert_eq!(dict.browse.query, "", "换词典清掉旧关键词");
        assert_eq!(dict.browse.page, 0);
        assert!(dict.browse.entries.is_empty(), "旧词典词条不该残留");
        assert!(!dict.browse.loaded, "新词典还没读过");
        assert!(dict.browse.loading, "切换应立刻发起首读");
        assert!(dict.browse.error.is_none());

        // 选同一本（pick_list 不会重发同名，防御性早退）。
        let before = dict.browse.loading;
        dict.browse_select("rime_ice".to_string());
        assert_eq!(dict.browse.loading, before, "同词典重复选择是空操作");
    }

    #[test]
    fn status_text_covers_loading_error_and_counts() {
        let mut browse = browse_with(rows(3), 0);
        browse.total = 42;
        browse.query = "gou".to_string();
        assert_eq!(browse.status_text(), "共 42 条词条，匹配 3 条");

        browse.query = String::new();
        assert_eq!(browse.status_text(), "共 42 条词条，显示前 3 条");

        browse.loading = true;
        assert_eq!(browse.status_text(), "正在读取词条…");

        browse.loading = false;
        browse.error = Some("读取词条失败（输入法服务未运行？）".to_string());
        assert_eq!(browse.status_text(), "读取词条失败（输入法服务未运行？）");

        let unread = DictBrowseState::default();
        assert_eq!(unread.status_text(), "尚未读取");
    }

    #[test]
    fn status_text_warns_when_hits_are_capped() {
        let mut browse = browse_with(rows(DICT_ENTRIES_MAX), 0);
        browse.matched = DICT_ENTRIES_MAX as i32 + 7;
        browse.query = "de".to_string();
        let text = browse.status_text();
        assert!(
            text.contains(&format!("匹配 {} 条", DICT_ENTRIES_MAX + 7)),
            "{text}"
        );
        assert!(
            text.contains(&format!("只列出前 {DICT_ENTRIES_MAX} 条")),
            "{text}"
        );
    }
}

/// 词条写入 / 方案词表 / 快捷短语的纯逻辑（校验、两步确认、翻页）。
#[cfg(all(test, windows))]
mod dict_edit_tests {
    use super::*;

    /// 新增对话框的校验与提交（服务端回调未注册时线程返回失败结果，
    /// 只断言提交前的同步状态：对话框关没关、writing 置位与否）。
    #[test]
    fn browse_add_submit_validates_input_locally() {
        let mut dict = DictManageState {
            browse: DictBrowseState {
                dict: "wubi86".to_string(),
                ..DictBrowseState::default()
            },
            ..DictManageState::default()
        };

        // 词/编码为空 → 报错并保留对话框。
        dict.browse_add_open();
        dict.browse_add_submit();
        assert!(dict.browse.add_dialog.is_some());
        assert_eq!(dict.browse.error.as_deref(), Some("词和编码都要填"));

        // 填上词和编码、频率留空（= 1）→ 提交成功：关对话框、置 writing。
        if let Some(draft) = dict.browse.add_dialog.as_mut() {
            draft.word = "曦码".to_string();
            draft.code = "jhdm".to_string();
        }
        dict.browse_add_submit();
        assert!(dict.browse.add_dialog.is_none());
        assert!(dict.browse.writing);
        assert!(dict.browse.error.is_none());
    }

    /// 频率输入的解析：空串 = 1；非正整数/非数字报错。
    #[test]
    fn parse_commits_input_blank_is_one_and_rejects_junk() {
        assert_eq!(parse_commits_input(""), Ok(1));
        assert_eq!(parse_commits_input("  7 "), Ok(7));
        assert!(parse_commits_input("0").is_err());
        assert!(parse_commits_input("-3").is_err());
        assert!(parse_commits_input("abc").is_err());
        assert!(parse_commits_input("1.5").is_err());
    }

    /// 权重输入的解析：空串 = None（省略该列）；非正整数报错。
    #[test]
    fn parse_weight_input_blank_is_none_and_rejects_junk() {
        assert_eq!(parse_weight_input(""), Ok(None));
        assert_eq!(parse_weight_input(" 99 "), Ok(Some(99)));
        assert!(parse_weight_input("0").is_err());
        assert!(parse_weight_input("-2").is_err());
        assert!(parse_weight_input("x").is_err());
    }

    /// 删除两步确认：请求 → 确认态；确认 → 清确认态并进入写入。
    #[test]
    fn browse_delete_two_step_confirmation() {
        let mut dict = DictManageState {
            browse: DictBrowseState {
                dict: "wubi86".to_string(),
                entries: vec![DictEntryRow {
                    word: "词".to_string(),
                    code: "code".to_string(),
                    commits: 3,
                }],
                ..DictBrowseState::default()
            },
            ..DictManageState::default()
        };

        dict.browse_delete_request("词".to_string(), "code".to_string());
        assert_eq!(
            dict.browse.delete_confirm.clone(),
            Some(("词".to_string(), "code".to_string()))
        );

        dict.browse_delete_cancel();
        assert!(dict.browse.delete_confirm.is_none());

        dict.browse_delete_request("词".to_string(), "code".to_string());
        dict.browse_delete_confirm("词".to_string(), "code".to_string());
        assert!(dict.browse.delete_confirm.is_none(), "确认后退出确认态");
        assert!(dict.browse.writing, "确认后进入写入");
    }

    /// 快捷短语对话框：校验失败保留对话框；成功关对话框并排队整表保存。
    #[test]
    fn phrase_dialog_submit_validates_and_enqueues_save() {
        let mut dict = DictManageState::default();
        dict.phrase_open("wubi86".to_string(), "五笔86".to_string());

        // 编码为空 → 报错保留对话框。
        dict.phrase_add_open();
        if let Some(phrase) = dict.phrase.as_mut() {
            if let Some(dialog) = phrase.dialog.as_mut() {
                dialog.word = "你好".to_string();
            }
        }
        dict.phrase_dialog_submit();
        assert!(dict
            .phrase
            .as_ref()
            .and_then(|p| p.dialog.clone())
            .is_some());
        assert_eq!(
            dict.phrase.as_ref().and_then(|p| p.error.clone()),
            Some("词和编码都要填".to_string())
        );

        // 补上编码 → 提交成功：关对话框、本地表多一条、进入保存。
        dict.phrase_dialog_word("你好".to_string());
        dict.phrase_dialog_code("lh".to_string());
        dict.phrase_dialog_weight("99".to_string());
        dict.phrase_dialog_submit();
        let Some(phrase) = dict.phrase.as_ref() else {
            panic!();
        };
        assert!(phrase.dialog.is_none());
        assert!(phrase.saving);
        assert_eq!(phrase.entries.len(), 1);
        assert_eq!(phrase.entries[0].word, "你好");
        assert_eq!(phrase.entries[0].code, "lh");
        assert_eq!(phrase.entries[0].weight, Some(99));
    }

    /// 快捷短语删除两步确认：确认后本地表少一条并排队保存。
    #[test]
    fn phrase_delete_two_step_confirmation() {
        let mut dict = DictManageState::default();
        dict.phrase_open("wubi86".to_string(), "五笔86".to_string());
        if let Some(phrase) = dict.phrase.as_mut() {
            phrase.entries = vec![
                CustomPhraseRow {
                    word: "甲".to_string(),
                    code: "a".to_string(),
                    weight: None,
                },
                CustomPhraseRow {
                    word: "乙".to_string(),
                    code: "b".to_string(),
                    weight: None,
                },
            ];
        }

        dict.phrase_delete_request(1);
        assert_eq!(dict.phrase.as_ref().and_then(|p| p.delete_confirm), Some(1));

        dict.phrase_delete_cancel();
        assert_eq!(dict.phrase.as_ref().and_then(|p| p.delete_confirm), None);

        dict.phrase_delete_request(1);
        dict.phrase_delete_confirm(1);
        let Some(phrase) = dict.phrase.as_ref() else {
            panic!();
        };
        assert_eq!(phrase.entries.len(), 1);
        assert_eq!(phrase.entries[0].word, "甲");
        assert!(phrase.saving);
    }

    /// 方案词表分页/状态（与用户词典词条浏览同款数学）。
    #[test]
    fn schema_dict_pagination_and_status() {
        let mut dict = SchemaDictState {
            schema_id: "wubi86".to_string(),
            entries: (0..120)
                .map(|i| DictEntryRow {
                    word: format!("词{i}"),
                    code: format!("code{i}"),
                    commits: 0,
                })
                .collect(),
            loaded: true,
            ..SchemaDictState::default()
        };
        assert_eq!(dict.page_count(), 3);
        assert_eq!(dict.page_entries().len(), DICT_ENTRIES_PAGE_SIZE);
        dict.page = 2;
        assert_eq!(dict.page_entries().len(), 20);

        // 状态文案：无关键词显示总数，有关键词显示命中数。
        dict.total = 42;
        assert!(dict.status_text().contains("共 42 条词条"));
        dict.query = "de".to_string();
        dict.matched = 9;
        assert!(dict.status_text().contains("匹配 9 条"));
    }

    /// 方案词表：命中超出回传上限时提示补充关键词。
    #[test]
    fn schema_dict_truncation_warning() {
        let dict = SchemaDictState {
            schema_id: "wubi86".to_string(),
            query: "de".to_string(),
            entries: vec![DictEntryRow {
                word: "词".to_string(),
                code: "code".to_string(),
                commits: 0,
            }],
            matched: DICT_ENTRIES_MAX as i32 + 3,
            loaded: true,
            ..SchemaDictState::default()
        };
        assert!(dict.truncated());
        assert!(dict
            .status_text()
            .contains(&format!("只列出前 {DICT_ENTRIES_MAX} 条")));
    }
}
