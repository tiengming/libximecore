//! 方案包清单与注册表（对齐 Android Xime 的 `SchemaManifestManager`）。
//!
//! Windows 侧把安卓的 `.manifests/<pkg>.json` + `.registry.json` 收敛为数据根下的
//! 一份 `<data_root>/.registry.yaml`：
//!
//! ```yaml
//! builtin:
//!   files:
//!   - wubi86.schema.yaml
//!   sha256:
//!     wubi86.schema.yaml: "<hex>"
//! rime-ice:
//!   files:
//!   - rime_ice.schema.yaml
//!   sha256:
//!     rime_ice.schema.yaml: "<hex>"
//! ```
//!
//! 与安卓一致的语义：
//! - `builtin` 包 = rime 目录里不属于任何市场包的方案文件（内置方案包），
//!   备份在 `market/builtin/`（对齐 `ensureBuiltinBackup` / `copyToBuiltinBackup`）
//! - 市场包不得静默覆盖其他包（含 builtin）的文件：内容不同即冲突
//!   （对齐 `detectConflicts`；同内容视为共享依赖放行）
//! - 卸载按 claimedBy 判断：仍有其他包声明该文件时只撤声明、不删文件
//!   （对齐 `uninstallWithManifest`），并清理该方案衍生产物：`<id>.custom.yaml`、
//!   `<id>_merged.dict.yaml`、方案短语表、`build/<id>.*`——rime 目录里不留上一个
//!   方案的任何内容
//! - 用户数据（`*.userdb/`、`installation.yaml`、`build/`、`themes/`、`sync/`）
//!   与宿主自有配置（`default.yaml` / `xime.yaml` / `custom_phrase.txt` / 其他平台
//!   前端配置）不入清单、不覆盖（对齐 `isUserDataFile` / `isProtectedSystemFile`）；
//!   其中 `custom_phrase.txt` 作为方案短语表在卸载该方案时一并删除，
//!   但市场包不得覆盖它（对齐安卓 `isProtectedImportName`）

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 内置方案包 id（对齐安卓 `SchemaManifestManager.BUILTIN_PACKAGE_ID`）。
pub const BUILTIN_PACKAGE_ID: &str = "builtin";

/// 用户手动放进 rime 目录、不属于任何已注册包的文件在 UI 里的归属。
/// 只用于展示与「删除本地方案」，不参与默认注入。
pub const LOCAL_PACKAGE_ID: &str = "local";

/// 注册表文件名（数据根下）。
pub const REGISTRY_FILE_NAME: &str = ".registry.yaml";

/// 内置方案备份目录名（数据根下 `market/builtin/`）。
pub const BUILTIN_BACKUP_DIR_NAME: &str = "builtin";

/// 市场包不得释放的顶层文件：宿主/引擎自有配置（覆盖会破坏方案配置）与其他平台
/// 前端配置（squirrel=macOS / weasel=小狼毫）与用户短语表（对齐安卓
/// `isProtectedImportName` 的 default.yaml / xime.yaml / custom_phrase.txt）。
pub const PROTECTED_RELEASE_FILES: &[&str] = &[
    "default.yaml",
    "default.custom.yaml",
    "xime.yaml",
    "xime.custom.yaml",
    "user.yaml",
    "installation.yaml",
    "squirrel.yaml",
    "squirrel.custom.yaml",
    "weasel.yaml",
    "weasel.custom.yaml",
    "custom_phrase.txt",
];

/// 包 id → UI 展示名（内置包显示中文）。
pub fn package_label(package_id: &str) -> String {
    if package_id == BUILTIN_PACKAGE_ID {
        "内置方案包".to_string()
    } else if package_id == LOCAL_PACKAGE_ID {
        "本地方案".to_string()
    } else {
        package_id.to_string()
    }
}

/// 归档/清单里的相对路径规范化：反斜杠转 `/`，去 `./` 前缀。
pub fn normalize_rel(rel: &str) -> String {
    rel.trim_start_matches("./")
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

/// 是否为市场包不得释放的路径（顶层受保护文件、部署产物、用户词典目录、注册表）。
pub fn is_protected_release_path(rel: &str) -> bool {
    let p = normalize_rel(rel);
    if p.is_empty() {
        return true;
    }
    if p.starts_with("build/") || p.starts_with(".registry") {
        return true;
    }
    if p.split('/').any(|seg| seg.ends_with(".userdb")) {
        return true;
    }
    if !p.contains('/') {
        return PROTECTED_RELEASE_FILES.contains(&p.as_str());
    }
    false
}

/// 是否为用户数据路径（不入清单、卸载不删；对齐安卓 `isUserDataFile`）。
pub fn is_user_data_path(rel: &str) -> bool {
    let p = normalize_rel(rel);
    if p.is_empty() {
        return true;
    }
    if p.starts_with("build/") || p.starts_with("themes/") || p.starts_with("sync/") {
        return true;
    }
    if p.split('/').any(|seg| seg.ends_with(".userdb")) {
        return true;
    }
    let base = p.rsplit('/').next().unwrap_or("");
    base.ends_with(".custom.yaml")
        || base == "installation.yaml"
        || base == "custom_phrase.txt"
        // 配置备份/编辑器临时文件：绝不能当成方案包文件（否则会被登记成 builtin）。
        || base.contains(".custom.yaml.")
        || base.ends_with(".bak")
        || base.ends_with(".orig")
        || base.ends_with(".tmp")
        || base.ends_with(".swp")
        || base.ends_with('~')
}

/// 是否可归入包清单（既非受保护、也非用户数据）。
pub fn is_trackable_path(rel: &str) -> bool {
    !is_protected_release_path(rel) && !is_user_data_path(rel)
}

/// 单个方案包的文件清单。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageEntry {
    /// 包释放到 rime 目录的文件（相对路径，`/` 分隔，已排序去重）。
    #[serde(default)]
    pub files: Vec<String>,
    /// 文件内容 sha256（冲突检测 / 共享判断；旧注册表可能为空）。
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
}

impl PackageEntry {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn claims(&self, rel: &str) -> bool {
        self.files.iter().any(|f| f == rel)
    }

    pub fn hash_of(&self, rel: &str) -> Option<&str> {
        self.sha256.get(rel).map(String::as_str)
    }
}

/// 注册表：包 id → 清单。
pub type Registry = BTreeMap<String, PackageEntry>;

/// 安装冲突（对齐安卓 `FileConflictInfo`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    pub file: String,
    /// 已声明该文件的其他包（含 `builtin`）。
    pub owners: Vec<String>,
    /// 已登记的内容哈希（旧注册表可能为空 = 未知）。
    pub existing_sha256: String,
    /// 待安装文件的内容哈希。
    pub new_sha256: String,
}

impl FileConflict {
    /// 便于 UI 展示：`rime_ice.schema.yaml（已被 内置方案包 使用）`。
    pub fn describe(&self) -> String {
        let owners = self
            .owners
            .iter()
            .map(|p| package_label(p))
            .collect::<Vec<_>>()
            .join("、");
        format!("{}（已被 {} 使用）", self.file, owners)
    }
}

/// 卸载结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallOutcome {
    /// 被删除的文件数。
    pub deleted_files: usize,
    /// 因其他包仍在声明而保留的文件数。
    pub kept_shared: usize,
    /// 一并清理的衍生产物数（`<id>.custom.yaml` / `<id>_merged.dict.yaml` / `build/<id>.*`）。
    pub deleted_derived: usize,
    /// 该包名下的方案 id（供调用方从启用列表移除）。
    pub removed_schema_ids: Vec<String>,
}

/// 方案包清单管理器（rime 目录 + 数据根）。
#[derive(Debug, Clone)]
pub struct SchemaManifest {
    rime_dir: PathBuf,
    data_root: PathBuf,
}

impl SchemaManifest {
    pub fn new(rime_dir: impl Into<PathBuf>, data_root: impl Into<PathBuf>) -> Self {
        Self {
            rime_dir: rime_dir.into(),
            data_root: data_root.into(),
        }
    }

    pub fn rime_dir(&self) -> &Path {
        &self.rime_dir
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// 注册表路径（数据根 `.registry.yaml`，与安卓「注册表在数据根」一致）。
    pub fn registry_path(&self) -> PathBuf {
        self.data_root.join(REGISTRY_FILE_NAME)
    }

    /// 内置方案备份目录（`market/builtin/`）。
    pub fn builtin_backup_dir(&self) -> PathBuf {
        self.data_root.join("market").join(BUILTIN_BACKUP_DIR_NAME)
    }

    /// 用户显式「清理内置」后的标记文件：存在时 server 不再把 seed 复制/登记回
    /// builtin。放在数据根而不是注册表里，避免和包清单格式耦合。
    pub fn builtin_removed_marker(&self) -> PathBuf {
        self.data_root.join(".builtin-removed")
    }

    pub fn is_builtin_removed(&self) -> bool {
        self.builtin_removed_marker().is_file()
    }

    pub fn set_builtin_removed(&self, removed: bool) -> Result<(), String> {
        let path = self.builtin_removed_marker();
        if removed {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建数据目录失败 {}: {}", parent.display(), e))?;
            }
            std::fs::write(&path, b"1")
                .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("删除 {} 失败: {}", path.display(), e)),
            }
        }
    }

    /// 用安装包自带的 seed 目录重新界定 builtin，并把其余无主文件登记为 local。
    ///
    /// - 只认 seed 里存在且 rime 目录里也存在的文件；用户删掉的内置文件不会复活。
    /// - seed 文件备份的是 **seed 原版**（不是用户改过的当前文件），便于恢复。
    /// - 旧注册表里被误登记为 builtin 的用户文件转成 local，不删除原文件。
    pub fn seed_builtin_from(&self, seed_dirs: &[PathBuf]) -> Result<usize, String> {
        let mut seed: BTreeMap<String, PathBuf> = BTreeMap::new();
        for dir in seed_dirs {
            if !dir.is_dir() {
                continue;
            }
            for rel in collect_files_in(dir)? {
                if !is_trackable_path(&rel) || seed.contains_key(&rel) {
                    continue;
                }
                let path = dir.join(&rel);
                seed.insert(rel, path);
            }
        }

        let mut registry = self.load_registry();
        // 只认安装包 seed：旧 builtin 里不是 seed 的文件（历史遗留/用户手放）
        // 交给 local，不删原文件。
        let mut entry = PackageEntry::default();
        let mut added = 0usize;
        for (rel, src) in &seed {
            let dest = self.rime_dir.join(rel);
            if !dest.is_file() {
                continue;
            }
            let Some(hash) = sha256_file(&dest) else {
                continue;
            };
            added += 1;
            entry.files.push(rel.clone());
            entry.sha256.insert(rel.clone(), hash);
            self.copy_seed_to_backup(rel, src);
        }
        entry.files.sort();
        entry.files.dedup();

        // 备份只保留当前安装包 seed，保证「恢复内置」不会把用户手放方案带回来。
        self.prune_builtin_backup(&seed.keys().cloned().collect::<BTreeSet<String>>());
        if entry.files.is_empty() {
            registry.remove(BUILTIN_PACKAGE_ID);
        } else {
            registry.insert(BUILTIN_PACKAGE_ID.to_string(), entry);
        }
        self.rebuild_local_in(&mut registry)?;
        self.save_registry(&registry)?;
        Ok(added)
    }

    /// 把 rime 目录里未被任何包声明的可跟踪文件登记为 `local`（只登记、不动文件）。
    pub fn register_local_package(&self) -> Result<(), String> {
        let mut registry = self.load_registry();
        self.rebuild_local_in(&mut registry)?;
        self.save_registry(&registry)
    }

    fn rebuild_local_in(&self, registry: &mut Registry) -> Result<(), String> {
        let claimed: BTreeSet<String> = registry
            .iter()
            .filter(|(pkg, _)| pkg.as_str() != LOCAL_PACKAGE_ID)
            .flat_map(|(_, entry)| entry.files.iter().cloned())
            .collect();
        let mut local = PackageEntry::default();
        for rel in self.collect_files()? {
            if !is_trackable_path(&rel) || claimed.contains(&rel) {
                continue;
            }
            let path = self.rime_dir.join(&rel);
            if !path.is_file() {
                continue;
            }
            let Some(hash) = sha256_file(&path) else {
                continue;
            };
            local.files.push(rel.clone());
            local.sha256.insert(rel, hash);
        }
        local.files.sort();
        local.files.dedup();
        if local.files.is_empty() {
            registry.remove(LOCAL_PACKAGE_ID);
        } else {
            registry.insert(LOCAL_PACKAGE_ID.to_string(), local);
        }
        Ok(())
    }

    fn copy_seed_to_backup(&self, rel: &str, src: &Path) {
        let dest = self.builtin_backup_dir().join(rel);
        if let Some(parent) = dest.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let _ = std::fs::copy(src, &dest);
    }

    fn prune_builtin_backup(&self, keep: &BTreeSet<String>) {
        let backup = self.builtin_backup_dir();
        let Ok(files) = collect_files_in(&backup) else {
            return;
        };
        for rel in files {
            if keep.contains(&rel) {
                continue;
            }
            let path = backup.join(&rel);
            let _ = std::fs::remove_file(&path);
            self.prune_empty_dirs(path.parent());
        }
    }

    /// 读取注册表；文件缺失或损坏时返回空表（对齐安卓 `loadRegistry` 容错）。
    pub fn load_registry(&self) -> Registry {
        let path = self.registry_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Registry::new();
        };
        match serde_yaml::from_str::<Registry>(&content) {
            Ok(registry) => registry,
            Err(e) => {
                tracing::warn!("registry {} corrupted, resetting: {}", path.display(), e);
                Registry::new()
            }
        }
    }

    pub fn save_registry(&self, registry: &Registry) -> Result<(), String> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建数据目录失败 {}: {}", parent.display(), e))?;
        }
        let yaml =
            serde_yaml::to_string(registry).map_err(|e| format!("序列化注册表失败: {}", e))?;
        std::fs::write(&path, yaml).map_err(|e| format!("写入注册表失败 {}: {}", path.display(), e))
    }

    /// 已安装的方案包 id（已排序）。
    pub fn installed_packages(&self) -> Vec<String> {
        self.load_registry()
            .into_iter()
            .filter(|(_, entry)| !entry.is_empty())
            .map(|(pkg, _)| pkg)
            .collect()
    }

    pub fn is_installed(&self, package_id: &str) -> bool {
        self.load_registry()
            .get(package_id)
            .map(|e| !e.is_empty())
            .unwrap_or(false)
    }

    /// 声明某文件的所有包（`builtin` 亦在其中）。
    pub fn owners_of(registry: &Registry, rel: &str) -> Vec<String> {
        let rel = normalize_rel(rel);
        registry
            .iter()
            .filter(|(_, entry)| entry.claims(&rel))
            .map(|(pkg, _)| pkg.clone())
            .collect()
    }

    /// 文件归属包：优先非内置包（共享文件在 UI 上更该显示第三方来源）。
    pub fn package_of(registry: &Registry, rel: &str) -> Option<String> {
        let owners = Self::owners_of(registry, rel);
        owners
            .iter()
            .find(|p| p.as_str() != BUILTIN_PACKAGE_ID)
            .cloned()
            .or_else(|| owners.first().cloned())
    }

    /// 清理 builtin 里已消失/已不可跟踪的声明（**不再**把无主文件认领成 builtin）。
    /// 真实登记由 server 启动时的 `seed_builtin_from()` 完成。
    pub fn refresh_builtin_package(&self) -> Result<usize, String> {
        let mut registry = self.load_registry();
        let mut entry = registry.remove(BUILTIN_PACKAGE_ID).unwrap_or_default();
        entry.files.retain(|rel| {
            self.rime_dir.join(rel).is_file() && is_trackable_path(rel)
        });
        entry
            .sha256
            .retain(|rel, _| entry.files.iter().any(|f| f == rel));
        if entry.is_empty() {
            registry.remove(BUILTIN_PACKAGE_ID);
        } else {
            registry.insert(BUILTIN_PACKAGE_ID.to_string(), entry);
        }
        self.rebuild_local_in(&mut registry)?;
        self.save_registry(&registry)?;
        Ok(0)
    }

    /// 冲突检测（对齐安卓 `detectConflicts`）：目标文件已被其他包声明且内容不同即冲突；
    /// 同内容视为共享依赖放行；同包重装/升级放行（对齐安卓「claimants 含自身即跳过」）。
    pub fn detect_conflicts(
        &self,
        package_id: &str,
        targets: &[(String, String)],
    ) -> Vec<FileConflict> {
        let registry = self.load_registry();
        let mut conflicts = Vec::new();
        for (rel, new_hash) in targets {
            let rel = normalize_rel(rel);
            let mut owners = Vec::new();
            let mut existing = String::new();
            let mut self_claimed = false;
            for (pkg, entry) in &registry {
                if !entry.claims(&rel) {
                    continue;
                }
                if pkg == package_id {
                    self_claimed = true;
                    continue;
                }
                if existing.is_empty() {
                    existing = entry.hash_of(&rel).unwrap_or_default().to_string();
                }
                owners.push(pkg.clone());
            }
            // 同包重装/升级：不检查该文件（对齐安卓注释「同一方案重新安装/升级：不视为冲突」）
            if self_claimed || owners.is_empty() {
                continue;
            }
            // 已登记哈希为空 = 旧注册表未知内容：保守视为冲突。
            if !existing.is_empty() && existing.eq_ignore_ascii_case(new_hash) {
                continue;
            }
            conflicts.push(FileConflict {
                file: rel,
                owners,
                existing_sha256: existing,
                new_sha256: new_hash.clone(),
            });
        }
        conflicts
    }

    /// 写入/更新一个包的清单（对齐安卓 `createManifest`）。同包重装即覆盖旧清单。
    pub fn register_package(
        &self,
        package_id: &str,
        targets: &[(String, String)],
    ) -> Result<(), String> {
        let mut registry = self.load_registry();
        let mut entry = PackageEntry::default();
        for (rel, hash) in targets {
            let rel = normalize_rel(rel);
            if rel.is_empty() {
                continue;
            }
            if !entry.files.contains(&rel) {
                entry.files.push(rel.clone());
            }
            entry.sha256.insert(rel, hash.clone());
        }
        entry.files.sort();
        registry.insert(package_id.to_string(), entry);
        self.save_registry(&registry)
    }

    /// 按清单卸载一个包（对齐安卓 `uninstallWithManifest`）：
    /// 逐文件检查 claimedBy，仍有其他包声明则保留；随后清理衍生产物。
    pub fn uninstall_package(&self, package_id: &str) -> Result<UninstallOutcome, String> {
        let mut registry = self.load_registry();
        let Some(entry) = registry.remove(package_id) else {
            return Err(format!("方案包 {} 未安装", package_label(package_id)));
        };

        let mut outcome = UninstallOutcome::default();
        let still_claimed: BTreeSet<String> = registry
            .values()
            .flat_map(|e| e.files.iter().cloned())
            .collect();

        // 方案 id 与短语表名必须在删文件之前算好：`<id>.schema.yaml`/`<id>.custom.yaml`
        // 一旦删除就解析不出 `custom_phrase.user_dict` 了（安卓同样是先读后删，
        // `uninstallWithManifest` 305-308 行）。
        let schema_phrases: Vec<(String, String)> = schema_ids_of(&entry)
            .into_iter()
            .map(|id| {
                let phrase = format!("{}.txt", custom_phrase_dict_name(&self.rime_dir, &id));
                (id, phrase)
            })
            .collect();

        for rel in &entry.files {
            if still_claimed.contains(rel) {
                outcome.kept_shared += 1;
                continue;
            }
            let path = self.rime_dir.join(rel);
            if path.is_file() && std::fs::remove_file(&path).is_ok() {
                outcome.deleted_files += 1;
                self.prune_empty_dirs(path.parent());
            }
        }

        // 衍生产物：<id>.custom.yaml / <id>_merged.dict.yaml / 方案短语表 / build/<id>.*
        // 对齐安卓 `uninstallWithManifest`：这几类都随方案一起删，
        // rime 目录里不留上一个方案的任何内容。
        for (id, phrase) in &schema_phrases {
            outcome.removed_schema_ids.push(id.clone());
            for rel in [
                format!("{id}.custom.yaml"),
                format!("{id}_merged.dict.yaml"),
            ] {
                let path = self.rime_dir.join(&rel);
                if path.is_file() && std::fs::remove_file(&path).is_ok() {
                    outcome.deleted_derived += 1;
                }
            }
            // 方案的自定义短语表（默认 `custom_phrase.txt`，`custom_phrase.user_dict`
            // 声明别的名字时用那个）：没有其他包声明才删（对齐安卓 342-355 行）。
            if !still_claimed.contains(phrase) {
                let path = self.rime_dir.join(phrase);
                if path.is_file() && std::fs::remove_file(&path).is_ok() {
                    outcome.deleted_derived += 1;
                }
            }
            let build_dir = self.rime_dir.join("build");
            if let Ok(entries) = std::fs::read_dir(&build_dir) {
                for file in entries.flatten() {
                    let name = file.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("{id}."))
                        && file.path().is_file()
                        && std::fs::remove_file(file.path()).is_ok()
                    {
                        outcome.deleted_derived += 1;
                    }
                }
            }
        }

        self.save_registry(&registry)?;
        tracing::info!(
            "卸载方案包 {}：删除 {} 个文件、保留 {} 个共享文件、清理 {} 个衍生产物",
            package_id,
            outcome.deleted_files,
            outcome.kept_shared,
            outcome.deleted_derived
        );
        Ok(outcome)
    }

    /// 从 `market/builtin/` 备份还原内置方案包（卸载内置方案后可一键恢复）。
    pub fn restore_builtin_package(&self) -> Result<usize, String> {
        let backup = self.builtin_backup_dir();
        if !backup.is_dir() {
            return Err("没有内置方案备份（market/builtin/ 不存在）".to_string());
        }
        let files = collect_files_in(&backup)?;
        if files.is_empty() {
            return Err("内置方案备份为空".to_string());
        }

        let mut targets = Vec::new();
        for rel in files {
            if !is_trackable_path(&rel) {
                continue;
            }
            let src = backup.join(&rel);
            let dest = self.rime_dir.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
            }
            std::fs::copy(&src, &dest).map_err(|e| format!("还原 {} 失败: {}", rel, e))?;
            targets.push((rel, sha256_file(&dest).unwrap_or_default()));
        }
        if targets.is_empty() {
            return Err("内置方案备份为空".to_string());
        }
        self.register_package(BUILTIN_PACKAGE_ID, &targets)?;
        tracing::info!("已从 market/builtin/ 还原 {} 个内置方案文件", targets.len());
        Ok(targets.len())
    }

    /// rime 目录下全部文件（相对路径，已排序）。
    pub fn collect_files(&self) -> Result<Vec<String>, String> {
        collect_files_in(&self.rime_dir)
    }

    /// 内置方案文件列表（备份目录内容，供 UI 判断「可还原」）。
    pub fn builtin_backup_files(&self) -> Vec<String> {
        collect_files_in(&self.builtin_backup_dir()).unwrap_or_default()
    }

    fn prune_empty_dirs(&self, dir: Option<&Path>) {
        let mut current = dir;
        while let Some(d) = current {
            if d == self.rime_dir || !d.starts_with(&self.rime_dir) {
                break;
            }
            match std::fs::read_dir(d) {
                Ok(mut entries) => {
                    if entries.next().is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
            if std::fs::remove_dir(d).is_err() {
                break;
            }
            current = d.parent();
        }
    }
}

/// 计算文件 sha256（十六进制小写）；读取失败返回 None。
pub fn sha256_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex::encode(hasher.finalize()))
}

/// 从包清单提取顶层方案 id（`*.schema.yaml` 的文件名主干）。
pub fn schema_ids_of(entry: &PackageEntry) -> Vec<String> {
    let mut ids: Vec<String> = entry
        .files
        .iter()
        .filter(|rel| !rel.contains('/'))
        .filter_map(|rel| rel.strip_suffix(".schema.yaml").map(String::from))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 递归收集目录下全部文件（相对路径 `/` 分隔，已排序）。
pub fn collect_files_in(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    collect_files_into(root, root, &mut out)?;
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_files_into(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("读取目录失败 {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_files_into(root, &path, out)?;
        } else if file_type.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// 方案的自定义短语表名（对齐安卓 `PersonalDictManager.getCustomPhraseDictName`）：
/// 先看 `<id>.custom.yaml`、再看 `<id>.schema.yaml` 里 `custom_phrase` 的 `user_dict`，
/// 都没有则用默认名 `custom_phrase`（即 `custom_phrase.txt`）。
///
/// pub：server 侧的快捷短语读写（`custom_phrase.txt` + 方案 patch）也用它解析表名，
/// 保证两边对"这张表叫什么"始终是同一个答案。
pub fn custom_phrase_dict_name(rime_dir: &Path, schema_id: &str) -> String {
    for rel in [
        format!("{schema_id}.custom.yaml"),
        format!("{schema_id}.schema.yaml"),
    ] {
        let Ok(text) = std::fs::read_to_string(rime_dir.join(&rel)) else {
            continue;
        };
        if let Some(name) = parse_custom_phrase_dict_name(&text) {
            return name;
        }
    }
    "custom_phrase".to_string()
}

/// 从配置文本里解析 `custom_phrase.user_dict` 声明的短语表文件名。
/// 支持三种写法：块式 `custom_phrase:` + `user_dict: name`、行内
/// `custom_phrase: { user_dict: name }`、补丁式 `custom_phrase/user_dict: name`
/// （安卓只认前两种，这里多认补丁式——rime 里覆盖短语表正是这么写的）。
fn parse_custom_phrase_dict_name(text: &str) -> Option<String> {
    // 补丁式：custom_phrase/user_dict: <name>
    for key in ["custom_phrase/user_dict", "\"custom_phrase\"/user_dict"] {
        if let Some(idx) = text.find(key) {
            let value = text[idx..]
                .split_once(':')
                .map(|(_, v)| phrase_value(v))
                .unwrap_or_default();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    // 块式 / 行内式：custom_phrase: { user_dict: <name> }
    for key in ["\"custom_phrase\"", "'custom_phrase'", "custom_phrase:"] {
        let Some(idx) = text.find(key) else {
            continue;
        };
        let after = &text[idx..];
        let Some(user_dict) = after.find("user_dict") else {
            continue;
        };
        let line = after[user_dict..].lines().next().unwrap_or("");
        let value = line
            .split_once(':')
            .map(|(_, v)| phrase_value(v))
            .unwrap_or_default();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

/// 短语表名取值：去掉注释、行内 YAML 的收尾符号与引号。
fn phrase_value(raw: &str) -> String {
    raw.split(['#', '}', ','])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\''])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, SchemaManifest) {
        let root = std::env::temp_dir().join(format!(
            "xime_manifest_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&root);
        let rime = root.join("rime");
        std::fs::create_dir_all(&rime).unwrap();
        (root.clone(), SchemaManifest::new(rime, root))
    }

    fn write_file(m: &SchemaManifest, rel: &str, content: &str) {
        let path = m.rime_dir().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn hash_of(content: &str) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(content.as_bytes()))
    }

    /// 测试里用 rime 目录本身当 seed（目录里写什么就视为随包内置的文件）。
    fn seed_from_rime(m: &SchemaManifest) -> Result<usize, String> {
        m.seed_builtin_from(&[m.rime_dir().to_path_buf()])
    }

    /// 内置包登记：只收方案文件，用户数据与宿主自有配置不入清单，并留备份。
    #[test]
    fn refresh_builtin_tracks_scheme_files_and_skips_user_data() {
        let (root, m) = fixture("builtin_refresh");
        write_file(&m, "wubi86.schema.yaml", "wubi86");
        write_file(&m, "wubi86.dict.yaml", "dict");
        write_file(&m, "lua/date_translator.lua", "lua");
        write_file(&m, "default.yaml", "host");
        write_file(&m, "user.yaml", "host");
        write_file(&m, "default.custom.yaml", "user");
        write_file(&m, "custom_phrase.txt", "user");
        write_file(&m, "installation.yaml", "user");
        write_file(&m, "build/wubi86.schema.yaml", "compiled");
        write_file(&m, "wubi86.userdb/db.txt", "userdb");
        write_file(&m, "sync/abc/wubi86.userdb.txt", "sync");

        let added = seed_from_rime(&m).unwrap();
        assert_eq!(added, 3, "仅 3 个方案文件应归入内置包");

        let registry = m.load_registry();
        let builtin = registry.get(BUILTIN_PACKAGE_ID).expect("builtin 应被登记");
        assert_eq!(
            builtin.files,
            vec![
                "lua/date_translator.lua".to_string(),
                "wubi86.dict.yaml".to_string(),
                "wubi86.schema.yaml".to_string(),
            ]
        );
        assert_eq!(
            builtin.hash_of("wubi86.schema.yaml"),
            Some(hash_of("wubi86").as_str()),
            "清单应记录内容哈希"
        );

        // 备份到 market/builtin/（对齐 copyToBuiltinBackup）
        let backup = m.builtin_backup_dir();
        assert!(backup.join("wubi86.schema.yaml").is_file());
        assert!(backup.join("lua/date_translator.lua").is_file());
        assert!(!backup.join("default.custom.yaml").exists());

        // 幂等：再刷新不新增
        assert_eq!(seed_from_rime(&m).unwrap(), 0);

        // 文件被手工删除后不再冒充内置方案文件
        std::fs::remove_file(m.rime_dir().join("lua/date_translator.lua")).unwrap();
        seed_from_rime(&m).unwrap();
        let builtin = m
            .load_registry()
            .get(BUILTIN_PACKAGE_ID)
            .cloned()
            .expect("builtin 仍在");
        assert!(!builtin.claims("lua/date_translator.lua"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// seed 只把随包文件登记为 builtin；用户手放方案归 local，配置备份不入清单。
    #[test]
    fn seed_marks_user_schema_as_local_not_builtin() {
        let (root, m) = fixture("seed_local");
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        std::fs::write(seed.join("builtin.schema.yaml"), "builtin").unwrap();
        write_file(&m, "builtin.schema.yaml", "builtin");
        write_file(&m, "my_own.schema.yaml", "user");
        write_file(&m, "xime.custom.yaml.bak", "backup");

        m.seed_builtin_from(&[seed]).unwrap();
        let registry = m.load_registry();
        let builtin = registry.get(BUILTIN_PACKAGE_ID).expect("builtin");
        assert_eq!(builtin.files, vec!["builtin.schema.yaml".to_string()]);
        let local = registry.get(LOCAL_PACKAGE_ID).expect("local");
        assert_eq!(local.files, vec!["my_own.schema.yaml".to_string()]);
        assert!(!local.claims("xime.custom.yaml.bak"));
        assert!(
            !m.builtin_backup_dir()
                .join("xime.custom.yaml.bak")
                .exists()
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 旧 builtin 里不是安装包 seed 的文件（历史遗留/用户手放）迁移到 local；
    /// 只有 seed 留下当内置，且原文件不删。
    #[test]
    fn seed_resets_builtin_to_seed_and_locals_legacy() {
        let (root, m) = fixture("migrate_local");
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        std::fs::write(seed.join("builtin.schema.yaml"), "builtin").unwrap();
        write_file(&m, "builtin.schema.yaml", "builtin");
        write_file(&m, "legacy.schema.yaml", "legacy");
        std::fs::write(
            m.registry_path(),
            "builtin:\n  files:\n  - builtin.schema.yaml\n  - legacy.schema.yaml\n",
        )
        .unwrap();

        m.seed_builtin_from(&[seed]).unwrap();
        let registry = m.load_registry();
        assert_eq!(
            registry.get(BUILTIN_PACKAGE_ID).map(|e| e.files.clone()),
            Some(vec!["builtin.schema.yaml".to_string()])
        );
        assert!(registry
            .get(LOCAL_PACKAGE_ID)
            .is_some_and(|e| e.claims("legacy.schema.yaml")));
        assert!(m.rime_dir().join("legacy.schema.yaml").is_file());

        // 迁移之后用户新放入的方案也归 local。
        write_file(&m, "new_user.schema.yaml", "user");
        m.register_local_package().unwrap();
        assert!(m
            .load_registry()
            .get(LOCAL_PACKAGE_ID)
            .is_some_and(|e| e.claims("new_user.schema.yaml")));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn builtin_removed_marker_roundtrip() {
        let (root, m) = fixture("builtin_marker");
        assert!(!m.is_builtin_removed());
        m.set_builtin_removed(true).unwrap();
        assert!(m.is_builtin_removed());
        m.set_builtin_removed(false).unwrap();
        assert!(!m.is_builtin_removed());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 冲突检测：同内容共享放行，异内容（含内置包占用）判冲突。
    #[test]
    fn detect_conflicts_allows_shared_content_and_flags_builtin() {
        let (root, m) = fixture("conflicts");
        write_file(&m, "wubi86.schema.yaml", "builtin-version");
        write_file(&m, "shared.dict.yaml", "shared");
        seed_from_rime(&m).unwrap();

        // 第三方包声明的 shared.dict.yaml 与内置包内容相同 → 共享依赖，放行
        let non_conflict = m.detect_conflicts(
            "rime-ice",
            &[("shared.dict.yaml".to_string(), hash_of("shared"))],
        );
        assert!(
            non_conflict.is_empty(),
            "同内容不应判冲突: {:?}",
            non_conflict
        );

        // 同名但内容不同（内置包已占用）→ 冲突，且指出归属
        let conflicts = m.detect_conflicts(
            "rime-ice",
            &[(
                "wubi86.schema.yaml".to_string(),
                hash_of("third-party-version"),
            )],
        );
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].file, "wubi86.schema.yaml");
        assert_eq!(conflicts[0].owners, vec![BUILTIN_PACKAGE_ID.to_string()]);
        assert!(conflicts[0].describe().contains("内置方案包"));

        // 同包重装不算冲突
        m.register_package(
            "rime-ice",
            &[(
                "wubi86.schema.yaml".to_string(),
                hash_of("third-party-version"),
            )],
        )
        .unwrap();
        let reinstall = m.detect_conflicts(
            "rime-ice",
            &[(
                "wubi86.schema.yaml".to_string(),
                hash_of("third-party-version"),
            )],
        );
        assert!(reinstall.is_empty(), "同包重装不应判冲突: {:?}", reinstall);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 卸载：共享文件保留、独占文件删除、衍生产物清理、方案 id 返回。
    /// 衍生产物（`<id>.custom.yaml` / 短语表 / `_merged` / `build/<id>.*`）随方案删除，
    /// 对齐安卓 `uninstallWithManifest`；输入记录（`*.userdb/`）保留。
    #[test]
    fn uninstall_keeps_shared_files_and_removes_derived() {
        let (root, m) = fixture("uninstall");
        write_file(&m, "a.schema.yaml", "a");
        write_file(&m, "a.custom.yaml", "user");
        write_file(&m, "a_merged.dict.yaml", "derived");
        write_file(&m, "custom_phrase.txt", "phrases");
        write_file(&m, "a.userdb/db.txt", "history");
        write_file(&m, "shared.dict.yaml", "shared");
        write_file(&m, "build/a.schema.yaml", "compiled");
        write_file(&m, "build/a.prism.bin", "bin");
        write_file(&m, "build/other.schema.yaml", "keep");

        m.register_package(
            "pkg-a",
            &[
                ("a.schema.yaml".to_string(), hash_of("a")),
                ("shared.dict.yaml".to_string(), hash_of("shared")),
            ],
        )
        .unwrap();
        m.register_package(
            "pkg-b",
            &[("shared.dict.yaml".to_string(), hash_of("shared"))],
        )
        .unwrap();

        let outcome = m.uninstall_package("pkg-a").unwrap();
        assert_eq!(outcome.deleted_files, 1, "仅 a.schema.yaml 归 pkg-a 独占");
        assert_eq!(outcome.kept_shared, 1, "shared.dict.yaml 仍被 pkg-b 声明");
        assert_eq!(
            outcome.deleted_derived, 5,
            "a.custom.yaml + custom_phrase.txt + a_merged.dict.yaml + build/a.* 两项"
        );
        assert_eq!(outcome.removed_schema_ids, vec!["a".to_string()]);

        assert!(!m.rime_dir().join("a.schema.yaml").exists());
        assert!(
            !m.rime_dir().join("a.custom.yaml").exists(),
            "补丁随方案删除"
        );
        assert!(
            !m.rime_dir().join("custom_phrase.txt").exists(),
            "短语表随方案删除"
        );
        assert!(!m.rime_dir().join("a_merged.dict.yaml").exists());
        assert!(!m.rime_dir().join("build/a.schema.yaml").exists());
        assert!(!m.rime_dir().join("build/a.prism.bin").exists());
        assert!(
            m.rime_dir().join("a.userdb/db.txt").is_file(),
            "输入记录保留"
        );
        assert!(m.rime_dir().join("shared.dict.yaml").is_file());
        assert!(m.rime_dir().join("build/other.schema.yaml").is_file());

        let registry = m.load_registry();
        assert!(!registry.contains_key("pkg-a"));
        assert!(registry
            .get("pkg-b")
            .map(|e| e.claims("shared.dict.yaml"))
            .unwrap_or(false));

        // 重复卸载报错而非静默
        assert!(m.uninstall_package("pkg-a").is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 旧注册表（无 sha256）仍可解析；内容未知时保守判冲突。
    #[test]
    fn legacy_registry_without_sha256_still_parses() {
        let (root, m) = fixture("legacy");
        write_file(&m, "wubi86.schema.yaml", "builtin");
        std::fs::write(
            m.registry_path(),
            "legacy-pkg:\n  files:\n  - wubi86.schema.yaml\n",
        )
        .unwrap();

        let registry = m.load_registry();
        let legacy = registry.get("legacy-pkg").expect("旧格式应能解析");
        assert!(legacy.claims("wubi86.schema.yaml"));
        assert!(legacy.sha256.is_empty());

        let conflicts = m.detect_conflicts(
            "rime-ice",
            &[("wubi86.schema.yaml".to_string(), hash_of("builtin"))],
        );
        assert_eq!(conflicts.len(), 1, "哈希未知时应保守判冲突");
        assert!(conflicts[0].existing_sha256.is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 卸载内置包后可从未动过的 market/builtin/ 备份还原。
    #[test]
    fn builtin_package_can_be_restored_from_backup() {
        let (root, m) = fixture("restore");
        write_file(&m, "wubi86.schema.yaml", "wubi86");
        write_file(&m, "lua/date_translator.lua", "lua");
        seed_from_rime(&m).unwrap();
        assert_eq!(m.builtin_backup_files().len(), 2);

        let outcome = m.uninstall_package(BUILTIN_PACKAGE_ID).unwrap();
        assert_eq!(outcome.deleted_files, 2);
        assert!(!m.is_installed(BUILTIN_PACKAGE_ID));
        assert!(!m.rime_dir().join("wubi86.schema.yaml").exists());
        assert!(m.builtin_backup_dir().join("wubi86.schema.yaml").is_file());

        let restored = m.restore_builtin_package().unwrap();
        assert_eq!(restored, 2);
        assert!(m.rime_dir().join("wubi86.schema.yaml").is_file());
        assert!(m.rime_dir().join("lua/date_translator.lua").is_file());
        let builtin = m
            .load_registry()
            .get(BUILTIN_PACKAGE_ID)
            .cloned()
            .expect("还原后应重新登记");
        assert!(builtin.claims("lua/date_translator.lua"));
        assert_eq!(
            builtin.hash_of("wubi86.schema.yaml"),
            Some(hash_of("wubi86").as_str())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 方案来源互斥（用户真实场景回归）：rime 目录里先有内置方案文件，又装了第三方包
    /// （历史混装）→ 卸载其中一个来源后，rime 目录里只剩另一个来源的文件，
    /// 用户数据（输入记录、自己写的补丁）与受保护文件不受影响。
    #[test]
    fn mixed_sources_converge_to_single_source() {
        let (root, m) = fixture("mixed_sources");
        // 内置方案包内容（含宿主受保护文件、用户补丁、用户词库）
        write_file(&m, "wubi86.schema.yaml", "builtin-wubi86");
        write_file(&m, "wubi86.dict.yaml", "builtin-dict");
        write_file(&m, "symbols.yaml", "shared-symbols");
        write_file(&m, "default.yaml", "host-config");
        write_file(&m, "wubi86.custom.yaml", "user-patch");
        write_file(&m, "wubi86.userdb/db.txt", "userdb");
        // seed 目录只放「随包内置」的三个文件；用户补丁/受保护文件不在其中。
        let seed = root.join("seed");
        std::fs::create_dir_all(seed.join("lua")).unwrap();
        std::fs::write(seed.join("wubi86.schema.yaml"), "builtin-wubi86").unwrap();
        std::fs::write(seed.join("wubi86.dict.yaml"), "builtin-dict").unwrap();
        std::fs::write(seed.join("symbols.yaml"), "shared-symbols").unwrap();
        assert_eq!(
            m.seed_builtin_from(&[seed]).unwrap(),
            3,
            "仅 seed 里的方案文件登记"
        );

        // 第三方包安装：自己的文件 + 与内置同内容的共享文件 symbols.yaml
        write_file(&m, "rime_ice.schema.yaml", "third-party");
        write_file(&m, "lua/date_translator.lua", "lua");
        m.register_package(
            "rime-ice",
            &[
                ("rime_ice.schema.yaml".to_string(), hash_of("third-party")),
                ("lua/date_translator.lua".to_string(), hash_of("lua")),
                ("symbols.yaml".to_string(), hash_of("shared-symbols")),
            ],
        )
        .unwrap();

        // 混装状态：注册表里两个非空来源
        let sources: Vec<String> = m
            .load_registry()
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(pkg, _)| pkg.clone())
            .collect();
        assert_eq!(sources.len(), 2, "内置 + 第三方 = 混装：{:?}", sources);

        // 收敛：只保留第三方来源 → 卸载内置方案包（对齐「装第三方方案时卸掉内置」）
        let outcome = m.uninstall_package(BUILTIN_PACKAGE_ID).unwrap();
        assert_eq!(outcome.deleted_files, 2, "wubi86.schema/dict.yaml 应被删除");
        assert_eq!(outcome.kept_shared, 1, "symbols.yaml 仍被第三方声明");
        assert!(!m.rime_dir().join("wubi86.schema.yaml").exists());
        assert!(!m.rime_dir().join("wubi86.dict.yaml").exists());
        // 上一个方案的补丁也一起走（对齐安卓），rime 目录不留它的内容
        assert!(!m.rime_dir().join("wubi86.custom.yaml").exists());
        // 共享文件、宿主受保护文件、输入记录都要留着
        assert!(m.rime_dir().join("symbols.yaml").is_file());
        assert!(m.rime_dir().join("default.yaml").is_file());
        assert!(m.rime_dir().join("wubi86.userdb/db.txt").is_file());

        let sources: Vec<String> = m
            .load_registry()
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(pkg, _)| pkg.clone())
            .collect();
        assert_eq!(sources, vec!["rime-ice".to_string()], "只剩一个来源");

        // 反向：卸载第三方来源 → 从备份还原内置方案，rime 目录回到只有内置一个来源
        m.uninstall_package("rime-ice").unwrap();
        assert!(!m.rime_dir().join("rime_ice.schema.yaml").exists());
        let restored = m.restore_builtin_package().unwrap();
        assert_eq!(restored, 3, "从 market/builtin/ 还原三个内置方案文件");
        assert!(m.rime_dir().join("wubi86.schema.yaml").is_file());
        let sources: Vec<String> = m
            .load_registry()
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(pkg, _)| pkg.clone())
            .collect();
        assert_eq!(
            sources,
            vec![BUILTIN_PACKAGE_ID.to_string()],
            "只剩内置来源"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 方案短语表名解析（对齐安卓 `getCustomPhraseDictName`）：
    /// `<id>.custom.yaml`/`<id>.schema.yaml` 里 `custom_phrase.user_dict` 声明优先。
    #[test]
    fn custom_phrase_dict_name_follows_user_dict_declaration() {
        let (root, m) = fixture("phrase_name");
        write_file(&m, "plain.schema.yaml", "schema_id: plain\n");
        write_file(
            &m,
            "double.schema.yaml",
            "schema_id: double\ncustom_phrase:\n  user_dict: custom_phrase_double\n",
        );
        write_file(
            &m,
            "quoted.schema.yaml",
            "schema_id: quoted\ncustom_phrase: { user_dict: \"custom_phrase_q\" }\n",
        );
        write_file(
            &m,
            "from_custom.schema.yaml",
            "schema_id: from_custom\ncustom_phrase:\n  user_dict: base_name\n",
        );
        write_file(
            &m,
            "from_custom.custom.yaml",
            "patch:\n  custom_phrase/user_dict: custom_phrase_patched\n",
        );

        assert_eq!(
            custom_phrase_dict_name(m.rime_dir(), "plain"),
            "custom_phrase"
        );
        assert_eq!(
            custom_phrase_dict_name(m.rime_dir(), "double"),
            "custom_phrase_double"
        );
        assert_eq!(
            custom_phrase_dict_name(m.rime_dir(), "quoted"),
            "custom_phrase_q"
        );
        assert_eq!(
            custom_phrase_dict_name(m.rime_dir(), "from_custom"),
            "custom_phrase_patched",
            ".custom.yaml 优先于 .schema.yaml"
        );

        // 卸载时该方案的短语表随之删除（默认名或声明名）
        write_file(&m, "custom_phrase_double.txt", "phrases");
        m.register_package(
            "pkg-double",
            &[(
                "double.schema.yaml".to_string(),
                hash_of("schema_id: double\ncustom_phrase:\n  user_dict: custom_phrase_double\n"),
            )],
        )
        .unwrap();
        m.uninstall_package("pkg-double").unwrap();
        assert!(!m.rime_dir().join("custom_phrase_double.txt").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 归属查询：共享文件优先显示第三方来源，未知文件不归属任何包。
    #[test]
    fn package_of_prefers_market_package() {
        let (root, m) = fixture("owners");
        write_file(&m, "shared.dict.yaml", "shared");
        write_file(&m, "wubi86.schema.yaml", "builtin");
        seed_from_rime(&m).unwrap();
        m.register_package(
            "rime-ice",
            &[("shared.dict.yaml".to_string(), hash_of("shared"))],
        )
        .unwrap();

        let registry = m.load_registry();
        assert_eq!(
            SchemaManifest::package_of(&registry, "shared.dict.yaml"),
            Some("rime-ice".to_string())
        );
        assert_eq!(
            SchemaManifest::package_of(&registry, "wubi86.schema.yaml"),
            Some(BUILTIN_PACKAGE_ID.to_string())
        );
        assert_eq!(SchemaManifest::package_of(&registry, "unknown.yaml"), None);
        assert!(SchemaManifest::owners_of(&registry, "shared.dict.yaml").len() == 2);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 路径规则：受保护项与用户数据不入清单。
    #[test]
    fn path_rules_match_android_semantics() {
        for rel in [
            "default.yaml",
            "xime.yaml",
            "custom_phrase.txt",
            "wubi86.custom.yaml",
            "build/wubi86.schema.yaml",
            "wubi86.userdb/db.txt",
        ] {
            assert!(!is_trackable_path(rel), "{rel} 不应入清单");
        }
        for rel in [
            "wubi86.schema.yaml",
            "lua/date_translator.lua",
            "cn_dicts/base.dict.yaml",
        ] {
            assert!(is_trackable_path(rel), "{rel} 应可入清单");
        }
        assert!(is_protected_release_path("custom_phrase.txt"));
        assert!(is_user_data_path("themes/bg.png"));
        assert!(is_user_data_path("sync/abc/wubi86.userdb.txt"));
    }
}
