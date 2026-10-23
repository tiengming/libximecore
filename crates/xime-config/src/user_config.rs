//! 用户配置文件（`xime.custom.yaml`）的读写规则。
//!
//! 与文档（`xime-docs/docs/features/keyboard-config-v2.md`）保持一致：
//!
//! - `xime.yaml` 是**随程序发布**的默认值，升级会被覆盖，程序**永远不写它**；
//! - 用户文件 `xime.custom.yaml` 只存**与默认值不同的部分**，加载时逐层深合并；
//! - 保存时**保留用户手写的其它键**（例如移动端才有的 `keyboard:` 段、或
//!   `metadata:` 这类我们结构体里没有的段），只改我们负责的顶层键——
//!   整份覆盖会把用户自己的东西删掉，这是之前最大的问题；
//! - 每次写入前留快照：`.bak`（撤销上一次修改）、`.orig`（用户最初的版本，
//!   只在第一次由程序改写一个「不是我们生成的」文件时留）。
//!
//! 语义上的「恢复默认」= 删掉用户文件（回到内置 + 系统级）；「恢复用户版本」
//! = 用 `.orig` 覆盖回去。两者都不动 `.bak`，撤销仍然可用。

use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// 文件头标记：判断这份文件是不是程序生成的。
///
/// 没有这个标记的文件 = 用户自己写的（例如从方案包里带来的），第一次程序改写它
/// 之前必须留一份 `.orig`，否则「恢复为用户提供的版本」就无从恢复。
pub const MARKER: &str = "# xime-user-config";

/// 用户配置文件的三种状态（设置页据此决定哪几个按钮可点）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfigFileState {
    /// 用户配置文件是否存在。
    pub user_file: bool,
    /// 是否有上一次修改的快照（可撤销）。
    pub backup: bool,
    /// 是否有用户最初的版本（可"恢复为用户提供的版本"）。
    pub original: bool,
}

impl ConfigFileState {
    /// 是否有任何可恢复的东西——没有就整块按钮都不显示。
    pub fn any(&self) -> bool {
        self.user_file || self.backup || self.original
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

pub fn backup_path(path: &Path) -> PathBuf {
    sibling(path, ".bak")
}

pub fn original_path(path: &Path) -> PathBuf {
    sibling(path, ".orig")
}

pub fn state_at(path: &Path) -> ConfigFileState {
    ConfigFileState {
        user_file: path.is_file(),
        backup: backup_path(path).is_file(),
        original: original_path(path).is_file(),
    }
}

/// 深合并：`over` 覆盖 `base`；两边都是映射时逐键递归，否则整体替换。
///
/// 这是"用户文件只写差异"能成立的前提：用户文件里没写的键，值来自默认值，
/// 而不是被结构体的 `[serde(default)]` 打回硬编码默认。
pub fn deep_merge(base: &mut Value, over: Value) {
    // `Null` 表示"没有差异/不覆盖"，不能把整棵树替换成 null。
    if over.is_null() {
        return;
    }
    match (base, over) {
        (Value::Mapping(base_map), Value::Mapping(over_map)) => {
            for (key, value) in over_map {
                match base_map.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

/// 计算 `value` 相对 `baseline` 的差异：只保留不同或新增的键。
///
/// 相同的子树整体返回 `Null`（调用方据此判断"没有差异"）。
pub fn deep_diff(value: &Value, baseline: &Value) -> Value {
    match (value, baseline) {
        (Value::Mapping(value_map), Value::Mapping(base_map)) => {
            let mut out = Mapping::new();
            for (key, val) in value_map {
                match base_map.get(key) {
                    Some(base_val) => {
                        let diff = deep_diff(val, base_val);
                        if !diff.is_null() {
                            out.insert(key.clone(), diff);
                        }
                    }
                    None => {
                        out.insert(key.clone(), val.clone());
                    }
                }
            }
            if out.is_empty() {
                Value::Null
            } else {
                Value::Mapping(out)
            }
        }
        (value, baseline) if value == baseline => Value::Null,
        (value, _) => value.clone(),
    }
}

fn read_tree(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_yaml::from_str::<Value>(&text).ok()
}

/// 写入用户文件：只改 `managed_keys` 这几个顶层键，其余原样保留。
///
/// `overrides` 是相对默认值的差异；某个受管键在差异里不存在 = 恢复默认，
/// 对应条目会从文件里被删掉（而不是留下一个钉死默认值的键）。
pub fn write_overrides_at(
    path: &Path,
    overrides: Value,
    managed_keys: &[String],
) -> Result<(), String> {
    let existing_text = fs::read_to_string(path).ok();
    // 快照要在改动**之前**做，且 .orig 只留第一次的用户版本。
    if let Some(text) = existing_text.as_deref() {
        let original = original_path(path);
        if !original.exists() && !text.contains(MARKER) {
            fs::write(&original, text)
                .map_err(|e| format!("写入 {} 失败: {e}", original.display()))?;
        }
        let backup = backup_path(path);
        fs::write(&backup, text).map_err(|e| format!("写入 {} 失败: {e}", backup.display()))?;
    }

    let mut tree = match read_tree(path) {
        Some(Value::Mapping(map)) => Value::Mapping(map),
        _ => Value::Mapping(Mapping::new()),
    };

    // 先摘掉我们负责的键，再合并差异：这样"改回默认"的键会真正消失。
    if let Value::Mapping(map) = &mut tree {
        for key in managed_keys {
            map.remove(Value::String(key.clone()));
        }
    }
    deep_merge(&mut tree, overrides);

    let empty = matches!(&tree, Value::Mapping(map) if map.is_empty());
    if empty {
        // 没有任何差异 = 回到"初始安装没有这个文件"的状态。
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
        }
        return Ok(());
    }

    let body = serde_yaml::to_string(&tree).map_err(|e| format!("序列化配置失败: {e}"))?;
    let content = format!(
        "{MARKER}\n# 只记录与内置默认值不同的部分；可直接编辑，重新部署后生效。\n{body}"
    );
    crate::atomic_write(path, content.as_bytes()).map_err(|e| format!("写入配置失败: {e}"))
}

/// 撤销上一次修改：用 `.bak` 覆盖回去。
pub fn undo_at(path: &Path) -> Result<(), String> {
    restore_from(&backup_path(path), path, "没有可撤销的备份")
}

/// 恢复为用户最初的版本（第一次被程序改写前的那份）。
pub fn restore_original_at(path: &Path) -> Result<(), String> {
    restore_from(&original_path(path), path, "没有用户初始版本可恢复")
}

fn restore_from(source: &Path, path: &Path, missing: &str) -> Result<(), String> {
    let text = fs::read_to_string(source).map_err(|_| format!("{missing}：{}", source.display()))?;
    crate::atomic_write(path, text.as_bytes()).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

/// 恢复为客户端默认：删掉用户文件（快照保留，撤销仍可用）。
pub fn restore_defaults_at(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    // 删之前也留一份 .bak，否则「恢复默认」就变成不可撤销的操作。
    if let Ok(text) = fs::read_to_string(path) {
        let backup = backup_path(path);
        let _ = fs::write(&backup, text);
    }
    fs::remove_file(path).map_err(|e| format!("删除 {} 失败: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xime-cfg-{}-{}",
            label,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn managed() -> Vec<String> {
        vec!["style".to_string(), "color_schemes".to_string()]
    }

    /// 保存只写差异：不把内置的 wubi_radicals / 全套配色灌进用户文件。
    #[test]
    fn write_overrides_keeps_only_diff() {
        let dir = temp_root("diff");
        let path = dir.join("xime.custom.yaml");
        let overrides: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        write_overrides_at(&path, overrides, &managed()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("font_size: 18"), "差异没写进去: {text}");
        assert!(!text.contains("wubi_radicals"), "不该整份 dump: {text}");
        assert!(!text.contains("color_schemes"), "不该整份 dump: {text}");
        assert!(text.contains(MARKER), "缺少文件头标记");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 用户手写的未知键必须保留（移动端的 keyboard 段等）。
    #[test]
    fn write_overrides_preserves_unknown_keys() {
        let dir = temp_root("keep");
        let path = dir.join("xime.custom.yaml");
        fs::write(&path, "keyboard:\n  rows: 3\nstyle:\n  font_size: 99.0\n").unwrap();

        let overrides: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        write_overrides_at(&path, overrides, &managed()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("keyboard"), "用户的其它段被删了: {text}");
        assert!(text.contains("rows: 3"), "用户的其它段被删了: {text}");
        assert!(text.contains("font_size: 18"), "受管键没更新: {text}");
        assert!(!text.contains("99"), "旧的受管键应被替换: {text}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 改回默认 = 该键从文件里消失；文件空了就删掉（回到"没有用户文件"）。
    #[test]
    fn write_overrides_removes_key_when_back_to_default() {
        let dir = temp_root("revert");
        let path = dir.join("xime.custom.yaml");
        let overrides: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        write_overrides_at(&path, overrides, &managed()).unwrap();
        assert!(path.exists());

        write_overrides_at(&path, Value::Null, &managed()).unwrap();
        assert!(!path.exists(), "没有差异时应删除用户文件");

        // 有未知键时不能整个删掉
        fs::write(&path, "keyboard:\n  rows: 3\n").unwrap();
        write_overrides_at(&path, Value::Null, &managed()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("rows: 3"), "未知键被误删: {text}");
        assert!(!text.contains("font_size"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// .orig 只在第一次改写「用户自己写的文件」时留一份，之后不再覆盖。
    #[test]
    fn original_snapshot_kept_once() {
        let dir = temp_root("orig");
        let path = dir.join("xime.custom.yaml");
        fs::write(&path, "style:\n  font_size: 20.0\n").unwrap();

        let o1: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        write_overrides_at(&path, o1, &managed()).unwrap();
        let orig = fs::read_to_string(original_path(&path)).unwrap();
        assert!(orig.contains("20"), "种子应是用户最初的内容: {orig}");

        let o2: Value = serde_yaml::from_str("style:\n  font_size: 22.0\n").unwrap();
        write_overrides_at(&path, o2, &managed()).unwrap();
        let orig = fs::read_to_string(original_path(&path)).unwrap();
        assert!(orig.contains("20"), "种子不能被后续写入覆盖: {orig}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 撤销 / 恢复用户版本 / 恢复默认三条路都要能回退到预期内容。
    #[test]
    fn undo_and_restore() {
        let dir = temp_root("undo");
        let path = dir.join("xime.custom.yaml");
        fs::write(&path, "style:\n  font_size: 20.0\n").unwrap();

        let o: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        write_overrides_at(&path, o, &managed()).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("18"));

        undo_at(&path).unwrap();
        assert!(
            fs::read_to_string(&path).unwrap().contains("20"),
            "撤销应回到上一次修改前"
        );

        restore_original_at(&path).unwrap();
        let restored = fs::read_to_string(&path).unwrap();
        assert!(restored.contains("20"), "{restored}");
        assert!(!restored.contains(MARKER), "用户版本不该带程序标记: {restored}");

        let state = state_at(&path);
        assert!(state.user_file && state.original, "状态应如实反映文件: {state:?}");

        restore_defaults_at(&path).unwrap();
        assert!(!path.exists(), "恢复默认应删除用户文件");
        assert!(state_at(&path).backup, "删之前要留撤销快照");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 深合并：用户只写 style.font_size 时，style 的其它字段仍取默认值。
    #[test]
    fn deep_merge_is_per_key() {
        let mut base: Value = serde_yaml::from_str(
            "style:\n  font_size: 14.0\n  candidate_count: 5\ncolor_schemes: {}\n",
        )
        .unwrap();
        let over: Value = serde_yaml::from_str("style:\n  font_size: 18.0\n").unwrap();
        deep_merge(&mut base, over);

        let font = base
            .get("style")
            .and_then(|s| s.get("font_size"))
            .and_then(|v| v.as_f64());
        assert_eq!(font, Some(18.0));
        let count = base
            .get("style")
            .and_then(|s| s.get("candidate_count"))
            .and_then(|v| v.as_i64());
        assert_eq!(count, Some(5), "未写的键必须保留默认值");
    }

    /// 差异计算：相同值不写、新增键写全量、嵌套只写变化的部分。
    #[test]
    fn deep_diff_rules() {
        let base: Value =
            serde_yaml::from_str("style:\n  font_size: 14.0\n  horizontal: true\n").unwrap();
        let same: Value =
            serde_yaml::from_str("style:\n  font_size: 14.0\n  horizontal: true\n").unwrap();
        assert!(deep_diff(&same, &base).is_null());

        let changed: Value =
            serde_yaml::from_str("style:\n  font_size: 18.0\n  horizontal: true\n").unwrap();
        let diff = deep_diff(&changed, &base);
        let text = serde_yaml::to_string(&diff).unwrap();
        assert!(text.contains("font_size: 18"), "{text}");
        assert!(!text.contains("horizontal"), "没变的键不该出现: {text}");
    }
}
