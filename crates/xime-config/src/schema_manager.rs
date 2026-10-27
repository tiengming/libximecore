use crate::rime_deploy::{get_data_dirs, init_rime_deployer, SchemaInfo};
use std::collections::HashSet;

fn replace_schema_list(content: &str, new_list: &str) -> String {
    if content.is_empty() {
        let mut result = String::from("patch:\n  schema_list:\n");
        for item in new_list.lines() {
            result.push_str(&format!("    {}\n", item.trim()));
        }
        return result;
    }

    let mut result = String::new();
    let mut replaced = false;
    let mut inserted_under_patch = false;
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // 已有 schema_list：整段替换。
        if line.trim() == "schema_list:" {
            replaced = true;
            i += 1;
            while i < lines.len()
                && lines[i].trim_start().starts_with('-')
                && lines[i].starts_with("    ")
            {
                i += 1;
            }
            result.push_str("  schema_list:\n");
            for item in new_list.lines() {
                result.push_str(&format!("    {}\n", item.trim()));
            }
            continue;
        }

        // 没有 schema_list 时插到已有 patch 下面，不能再追加一个顶层 patch：
        // YAML 出现重复 key 时 librime 只取第一个 patch，schema_list 会被静默丢掉，
        // 表现为部署后没有 build、方案全显示「未启用」。
        if !replaced && !inserted_under_patch && line.trim() == "patch:" {
            result.push_str(line);
            result.push('\n');
            result.push_str("  schema_list:\n");
            for item in new_list.lines() {
                result.push_str(&format!("    {}\n", item.trim()));
            }
            inserted_under_patch = true;
            i += 1;
            continue;
        }

        result.push_str(line);
        result.push('\n');
        i += 1;
    }

    if !replaced && !inserted_under_patch {
        result.push_str("\npatch:\n  schema_list:\n");
        for item in new_list.lines() {
            result.push_str(&format!("    {}\n", item.trim()));
        }
        result.push('\n');
    }

    result
}

/// 写 `default.custom.yaml` 里的 `patch.menu.page_size`（不需要初始化 librime）。
///
/// 单独拆成自由函数：服务端只想改这一个键，没必要为了它去 `initialize` rime。
/// 返回是否真的改动（已一致时返回 `false`，调用方可据此决定要不要部署）。
///
/// 只改默认层：方案自己的 `menu.page_size` 会压过它，所以**方案包里写死的 page_size
/// 要由方案自己删掉**（`xime-docs` 的约定：候选数量归 xime 管，方案不要写 page_size）。
pub fn set_page_size_in(user_dir: &std::path::Path, page_size: usize) -> Result<bool, String> {
    write_page_size_file(&user_dir.join("default.custom.yaml"), page_size)
}

/// `default.custom.yaml` 的文件头：这份文件归 Xime 管，用户不要手改。
const DEFAULT_CUSTOM_HEADER: &str = "# default.custom.yaml —— Xime固定配置（不随 assets/rime 方案 submodule 分发）\n\
# 安装/启动或设置变动时自动同步到用户 rime 目录，部署时 patch 到 default.yaml，\n\
# 请勿在此手工调整；其余 patch 节为默认配置基线，修改后随 app 更新生效。\n";

/// 只改 `patch.menu.page_size`，其余 YAML 原样保留；返回是否改动。
fn write_page_size_file(path: &std::path::Path, page_size: usize) -> Result<bool, String> {
    let page_size = page_size.clamp(1, 10);
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut tree: serde_yaml::Value =
        serde_yaml::from_str(&text).unwrap_or(serde_yaml::Value::Null);
    if !tree.is_mapping() {
        tree = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }

    let current = tree
        .get("patch")
        .and_then(|patch| patch.get("menu"))
        .and_then(|menu| menu.get("page_size"))
        .and_then(|value| value.as_i64());
    if current == Some(page_size as i64) {
        return Ok(false);
    }

    if let Some(patch) = ensure_child_mapping(&mut tree, "patch") {
        if let Some(menu) = ensure_child_mapping(patch, "menu") {
            if let Some(map) = menu.as_mapping_mut() {
                map.insert(
                    serde_yaml::Value::String("page_size".to_string()),
                    serde_yaml::Value::Number((page_size as u64).into()),
                );
            }
        }
    }

    let body = serde_yaml::to_string(&tree).map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    // `default.custom.yaml` 是 Xime 固定配置：写入时带上说明头，避免用户以为是自己的文件。
    let is_default_custom = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "default.custom.yaml")
        .unwrap_or(false);
    let content = if is_default_custom {
        format!("{DEFAULT_CUSTOM_HEADER}{body}")
    } else {
        body
    };
    crate::atomic_write(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(true)
}

pub struct SchemaManager {
    user_dir: std::path::PathBuf,
}

/// 取（必要时建）`parent[key]` 的映射，用来逐层写配置而不动其它键。
fn ensure_child_mapping<'a>(
    parent: &'a mut serde_yaml::Value,
    key: &str,
) -> Option<&'a mut serde_yaml::Value> {
    let map = parent.as_mapping_mut()?;
    let entry = map
        .entry(serde_yaml::Value::String(key.to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if !entry.is_mapping() {
        *entry = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    Some(entry)
}

impl SchemaManager {
    pub fn new() -> Result<Self, String> {
        init_rime_deployer()?;
        let (_, user_dir) = get_data_dirs();
        Ok(Self { user_dir })
    }

    /// 让 rime 的 `menu/page_size` 与候选栏显示数量一致（写 `default.custom.yaml`）。
    ///
    /// 候选来自 librime 的**一页**：`style.candidate_count` 想显示 7 个，
    /// `menu/page_size` 就必须 ≥ 7，否则候选栏最多只有 5 个可显示
    /// （「候选词数量」调大没反应，十有八九是这个）。只改这一个键，
    /// 文件里其它 patch 原样保留；返回是否真的改动（改动后要部署才生效）。
    pub fn set_page_size(&self, page_size: usize) -> Result<bool, String> {
        set_page_size_in(&self.user_dir, page_size)
    }

    pub fn get_schema_list(&self) -> Vec<SchemaInfo> {
        let (shared_data_dir, user_data_dir) = get_data_dirs();
        let mut schemas = Vec::new();
        let mut seen_ids = HashSet::new();

        if let Ok(entries) = std::fs::read_dir(&user_data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".schema.yaml") {
                        let schema_id = name.replace(".schema.yaml", "");

                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let schema_name = extract_schema_name(&content, &schema_id);
                            schemas.push(SchemaInfo {
                                schema_id: schema_id.clone(),
                                name: schema_name,
                            });
                            seen_ids.insert(schema_id);
                        }
                    }
                }
            }
        }

        if let Ok(entries) = std::fs::read_dir(&shared_data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".schema.yaml") {
                        let schema_id = name.replace(".schema.yaml", "");

                        if seen_ids.contains(&schema_id) {
                            continue;
                        }

                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let schema_name = extract_schema_name(&content, &schema_id);
                            schemas.push(SchemaInfo {
                                schema_id,
                                name: schema_name,
                            });
                        }
                    }
                }
            }
        }

        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        schemas
    }

    pub fn set_schema_list(&self, schema_ids: &[&str]) -> Result<(), String> {
        let default_custom = self.user_dir.join("default.custom.yaml");

        let new_list = schema_ids
            .iter()
            .map(|id| format!("- schema: {}", id))
            .collect::<Vec<_>>()
            .join("\n");

        let content = if default_custom.exists() {
            std::fs::read_to_string(&default_custom)
                .map_err(|e| format!("Failed to read default.custom.yaml: {}", e))?
        } else {
            let (shared_dir, _) = get_data_dirs();
            let source = shared_dir.join("default.custom.yaml");
            if source.exists() {
                std::fs::read_to_string(&source)
                    .map_err(|e| format!("Failed to read default.custom.yaml: {}", e))?
            } else {
                String::new()
            }
        };

        let updated = replace_schema_list(&content, &new_list);

        std::fs::write(&default_custom, updated)
            .map_err(|e| format!("Failed to write default.custom.yaml: {}", e))?;

        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        Ok(())
    }

    pub fn get_selected_schema(&self) -> Option<String> {
        let default_custom = self.user_dir.join("default.custom.yaml");
        if !default_custom.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&default_custom).ok()?;
        extract_selected_schema(&content)
    }

    /// 读取用户 `default.custom.yaml` 中 `schema_list` 的完整顺序。
    pub fn get_schema_list_ids(&self) -> Vec<String> {
        let default_custom = self.user_dir.join("default.custom.yaml");
        if !default_custom.exists() {
            return Vec::new();
        }
        let Ok(content) = std::fs::read_to_string(&default_custom) else {
            return Vec::new();
        };
        content.lines().filter_map(extract_schema_line).collect()
    }
}

fn extract_selected_schema(content: &str) -> Option<String> {
    for line in content.lines() {
        if line.contains("schema:") {
            let schema = line
                .split("schema:")
                .nth(1)
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string());
            if let Some(s) = schema {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// 从 `- schema: xxx` 行提取方案 id。
fn extract_schema_line(line: &str) -> Option<String> {
    let line = line.trim();
    let line = line.strip_prefix('-').unwrap_or(line).trim();
    if let Some(rest) = line.strip_prefix("schema:") {
        let id = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

fn extract_schema_name(content: &str, schema_id: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name:") {
            return trimmed
                .split(':')
                .nth(1)
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .unwrap_or_else(|| schema_id.to_string());
        }
    }
    schema_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已有 `patch.menu` 但没有 schema_list 时，schema_list 必须插进同一个 patch；
    /// 不能再追加第二个顶层 `patch:`（重复 key 会让 rime 丢列表）。
    #[test]
    fn schema_list_inserted_into_existing_patch() {
        let content = "# header\npatch:\n  menu:\n    page_size: 5\n";
        let updated = replace_schema_list(content, "- schema: wubi86_pinyin");
        assert_eq!(
            updated.matches("\npatch:").count(),
            1,
            "不允许出现重复 patch: {updated}"
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&updated).unwrap();
        let list = parsed
            .get("patch")
            .and_then(|patch| patch.get("schema_list"))
            .and_then(|list| list.as_sequence())
            .expect("schema_list 应可解析");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("schema").and_then(|v| v.as_str()), Some("wubi86_pinyin"));
        assert_eq!(
            parsed
                .get("patch")
                .and_then(|patch| patch.get("menu"))
                .and_then(|menu| menu.get("page_size"))
                .and_then(|v| v.as_i64()),
            Some(5)
        );
    }

    /// `menu/page_size` 落盘：只改这一个键，其它 patch 原样保留；已一致时不重复写。
    #[test]
    fn page_size_written_without_touching_other_patch_keys() {
        let dir = std::env::temp_dir().join(format!("xime_pagesize_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("default.custom.yaml");
        std::fs::write(
            &path,
            "patch:\n  schema_list:\n    - schema: moqi_wan_flypymo\n",
        )
        .unwrap();

        assert_eq!(set_page_size_in(&dir, 7), Ok(true));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# default.custom.yaml"), "缺少固定配置说明头: {text}");
        assert!(text.contains("page_size: 7"), "{text}");
        assert!(text.contains("moqi_wan_flypymo"), "其它 patch 被删: {text}");

        // 已经一致：返回 false，文件内容不变
        let before = std::fs::read_to_string(&path).unwrap();
        assert_eq!(set_page_size_in(&dir, 7), Ok(false));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        // 改成别的值：覆盖，且仍然保留 schema_list
        assert_eq!(set_page_size_in(&dir, 9), Ok(true));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("page_size: 9"), "{text}");
        assert!(text.contains("moqi_wan_flypymo"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_schema_list_uses_rime_wubi_not_system() {
        // 注入双目录（shared=带 fixture 方案的临时目录，user=空临时目录），
        // 验证：1) set_rime_paths/get_data_dirs roundtrip；2) get_schema_list 从注入目录枚举；
        // 3) 未写入的方案（如系统内置 stroke）不出现。fixture 保证测试不依赖机器状态。
        let base =
            std::env::temp_dir().join(format!("xime_config_schema_test_{}", std::process::id()));
        let shared = base.join("rime-data");
        let user = base.join("rime-user");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(
            shared.join("wubi86_pinyin.schema.yaml"),
            "schema:\n  schema_id: wubi86_pinyin\n  name: 五笔拼音\n  version: \"0.1\"\n",
        )
        .unwrap();

        let _ = crate::set_rime_paths(crate::RimePaths {
            shared_data_dir: shared.clone(),
            user_data_dir: user.clone(),
        });
        // set_rime_paths roundtrip
        let (s, u) = crate::get_data_dirs();
        assert_eq!(s, shared);
        assert_eq!(u, user);

        let manager = SchemaManager::new().unwrap();
        let schemas = manager.get_schema_list();
        let ids: Vec<_> = schemas.iter().map(|s| s.schema_id.as_str()).collect();
        // fixture 方案应存在
        assert!(
            ids.contains(&"wubi86_pinyin"),
            "wubi86_pinyin should be in list: {:?}",
            ids
        );
        // 未写入的方案不应出现
        assert!(
            !ids.contains(&"stroke"),
            "system schema stroke must not appear: {:?}",
            ids
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_extract_schema_line_and_list_roundtrip() {
        let yaml = "patch:\n  schema_list:\n    - schema: wubi86\n    - schema: \"luna_pinyin\"\n    - schema: 'cangjie'\n";
        let ids: Vec<String> = yaml.lines().filter_map(extract_schema_line).collect();
        assert_eq!(ids, vec!["wubi86", "luna_pinyin", "cangjie"]);

        // 顺序保持 + 去重逻辑配合 save_schema
        let mut list = ids;
        list.retain(|id| id != "wubi86");
        list.insert(0, "wubi86".to_string());
        assert_eq!(list, vec!["wubi86", "luna_pinyin", "cangjie"]);

        // 带前缀（非 schema 行）应被忽略
        assert_eq!(extract_schema_line("  other: 1"), None);
        assert_eq!(extract_schema_line("  - schema: \"\""), None);
    }
}
