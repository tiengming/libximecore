use crate::components::widgets::{
    badge, button_danger, button_disabled, button_primary, card_style, medium, pick_list_style,
    scroll_style_bare, semibold, switch, text_button, RADIUS_MD,
};
use crate::state::{
    MarketModel, MarketModelState, MarketPlugin, MarketPluginState, MarketSchema,
    MarketSchemaState, Message, SettingsState,
};
use crate::theme::ThemeColors;
use iced::widget::{button, column, container, pick_list, row, scrollable, text};
use iced::{border, Alignment, Background, Border, Color, Element, Length};

/// 模型分类 → 展示标签（对应索引中的 category 字段）。
const MODEL_CATEGORIES: [(&str, &str); 4] = [
    ("prediction", "联想"),
    ("handwriting", "手写"),
    ("asr", "语音"),
    ("other", "其他"),
];

/// 插件类型 → 展示标签（对应索引中的 pluginType 字段）。
const PLUGIN_CATEGORIES: [(&str, &str); 4] = [
    ("speech", "语音"),
    ("emoji", "表情"),
    ("prediction", "联想"),
    ("other", "其他"),
];

pub fn view<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let store = &settings.market_schema;
    let tab = store.store_tab;

    let mut content = column![header(colors), store_tab_bar(tab, colors),]
        .spacing(16)
        .padding(20)
        .width(Length::Fill);

    content = content.push(if tab == 0 {
        schemes_tab(settings, colors)
    } else if tab == 1 {
        models_tab(settings, colors)
    } else {
        plugins_tab(settings, colors)
    });

    content.into()
}

/// 页头：标题 + 刷新按钮。
fn header<'a>(colors: &'a ThemeColors) -> Element<'a, Message> {
    row![
        text("扩展商店")
            .size(20)
            .font(semibold())
            .color(colors.foreground)
            .width(Length::Fill),
        text_button("刷新", colors, Message::MarketRetry),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

/// 一级 Tab：方案 / 模型 / 插件。
fn store_tab_bar<'a>(active: usize, colors: &'a ThemeColors) -> Element<'a, Message> {
    crate::components::widgets::tab_bar(
        &["方案", "模型", "插件"],
        active,
        colors,
        Message::StoreTab,
    )
}

/// 分类筛选 chips（横向滚动），第一个恒为「全部」。
fn chip_bar<'a>(
    mut tags: Vec<String>,
    selected: &'a Option<String>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let mut options: Vec<String> = Vec::with_capacity(tags.len() + 1);
    options.push(String::new());
    options.append(&mut tags);

    let mut chips = row![].spacing(6).padding([2, 0]);
    for tag in options {
        let is_active = selected.as_deref() == Some(tag.as_str());
        chips = chips.push(chip(tag, is_active, colors));
    }

    scrollable(chips)
        .horizontal()
        .width(Length::Fill)
        .height(Length::Shrink)
        .style(move |theme, status| scroll_style_bare(colors, theme, status))
        .into()
}

fn chip<'a>(tag: String, active: bool, colors: &'a ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    let label = if tag.is_empty() {
        "全部".to_string()
    } else {
        tag.clone()
    };
    button(
        container(
            text(label)
                .size(13)
                .font(medium())
                .color(if active {
                    colors.on_primary
                } else {
                    colors.foreground_muted
                }),
        )
        .height(Length::Fill)
        .center_y(Length::Fill),
    )
    .height(30.0)
    .padding([0, 12])
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        // 未选中项用「白底 + 细描边」：筛选条本身贴在浅灰页面上，
        // 浅灰填充会和页面糊在一起，看着像一排纯文字。
        button::Style {
            background: Some(Background::Color(if active {
                colors.primary
            } else if hovered {
                colors.surface_hover
            } else {
                colors.control_bg
            })),
            text_color: if active {
                colors.on_primary
            } else {
                colors.foreground_muted
            },
            border: Border {
                color: if active {
                    Color::TRANSPARENT
                } else {
                    colors.control_border
                },
                width: if active { 0.0 } else { 1.0 },
                radius: border::radius(999.0),
            },
            ..button::Style::default()
        }
    })
    .on_press(Message::StoreTagSelected(tag))
    .into()
}

// ---- 方案 Tab ----

/// 把子元素排成 3 列的网格：每 3 个一行，行高统一，行内均分宽度。
/// 最后一行不足 3 个时，用透明占位补齐到 3 份，卡片保持单列宽度。
fn grid_rows<'a>(
    children: impl IntoIterator<Item = Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> Element<'a, Message, iced::Theme, iced::Renderer> {
    let cards: Vec<_> = children.into_iter().collect();
    let mut list = column![].spacing(12).width(Length::Fill);
    let mut iter = cards.into_iter();
    while iter.len() > 0 {
        let taken: Vec<_> = iter.by_ref().take(2).collect();
        let missing = 2 - taken.len();
        // 不写死行高：以前 `.height(160)` 会把带提示条的卡片直接裁掉一截
        // （提示文字只剩半个字高）。卡片按内容自适应，同排高度差由内容决定。
        let mut line = row![].spacing(12).width(Length::Fill);
        for card in taken {
            line = line.push(container(card).width(Length::FillPortion(1)));
        }
        for _ in 0..missing {
            line = line.push(iced::widget::Space::new().width(Length::FillPortion(1)));
        }
        list = list.push(line);
    }
    list.into()
}

fn schemes_tab<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let store = &settings.market_schema;

    let content: Element<'a, Message> = if store.loading && !store.loaded {
        center_msg(colors, "正在加载方案列表…")
    } else if let Some(error) = &store.error {
        error_view(colors, error)
    } else if store.loaded {
        schema_list(store, colors)
    } else {
        center_msg(colors, "正在加载方案列表…")
    };

    let mut column = column![content].spacing(10).width(Length::Fill);

    if let Some(msg) = &store.install_message {
        column = column.push(text(msg.clone()).size(13).color(colors.error));
    }
    if store.loaded {
        column = column.push(footer(
            store.schemas.len(),
            "方案",
            &store.updated_at,
            colors,
        ));
    }

    column.into()
}

fn schema_list<'a>(store: &'a MarketSchemaState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let mut tags: Vec<String> = Vec::new();
    for schema in &store.schemas {
        for tag in &schema.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
    }

    let schemas: Vec<&MarketSchema> = store
        .schemas
        .iter()
        .filter(|s| {
            store
                .selected_tag
                .as_ref()
                .map(|t| s.tags.contains(t))
                .unwrap_or(true)
        })
        .collect();

    if schemas.is_empty() {
        return center_msg(colors, "没有匹配的方案");
    }

    let mut list = column![].spacing(8).width(Length::Fill);
    if !tags.is_empty() {
        list = list.push(chip_bar(tags, &store.selected_tag, colors));
    }
    list = list.push(grid_rows(
        schemas
            .iter()
            .map(|schema| schema_card(schema, store, colors)),
    ));
    list.into()
}

fn schema_card<'a>(
    schema: &'a MarketSchema,
    store: &'a MarketSchemaState,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    // 已安装/已下载判定用归一化 id：注册表里记的可能是历史 key 或历史目录名，
    // 与索引 id 直接比字符串会漏判（表现为「明明装了却显示未安装」）。
    let installed = store
        .installed_ids
        .iter()
        .any(|id| crate::state::package_id_eq(id, &schema.id));
    let downloaded = store
        .downloaded_ids
        .iter()
        .any(|id| crate::state::package_id_eq(id, &schema.id));
    let downloading = store.downloading.as_deref() == Some(schema.id.as_str());
    let installing = store.installing.as_deref() == Some(schema.id.as_str());
    let progress = if downloading {
        store.download_progress
    } else {
        None
    };
    let selected_version = store.selected_versions.get(&schema.id).cloned();

    let versions: Vec<String> = schema.versions.iter().map(|v| v.version.clone()).collect();
    let version = selected_version
        .or_else(|| schema.current_version.clone())
        .unwrap_or_else(|| versions.first().cloned().unwrap_or_else(|| "latest".into()));

    let size_label = schema
        .versions
        .iter()
        .find(|v| v.version == version)
        .or_else(|| schema.versions.first())
        .and_then(|v| {
            v.download_url
                .first()
                .and_then(|u| u.size.as_deref())
                .or(v.size.as_deref())
                .map(format_size)
        })
        .unwrap_or_default();

    let glyph = schema
        .tags
        .first()
        .and_then(|t| t.chars().next())
        .or_else(|| schema.name.chars().next())
        .unwrap_or('方');

    let header = row![
        glyph_box(glyph, schema_icon_color(colors)),
        column![
            row![
                text(if schema.name.is_empty() {
                    schema.id.clone()
                } else {
                    schema.name.clone()
                })
                .size(15)
                .font(semibold())
                .color(colors.foreground),
                if schema.schema_type == "built-in" {
                    badge("内置", colors.on_primary, colors.primary)
                } else {
                    text("").size(15).into()
                },
                container(text("")).width(Length::Fill),
                if size_label.is_empty() {
                    text("").size(13)
                } else {
                    text(size_label).size(13).color(colors.foreground_muted)
                },
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            if schema.author.is_empty() && schema.tags.is_empty() {
                iced::widget::Space::new().height(12).into()
            } else {
                author_tags_row(&schema.author, &schema.tags, colors)
            },
            text(truncate(&schema.description, DESC_MAX_CHARS))
                .size(13)
                .color(colors.foreground_muted),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
    .width(Length::Fill);

    let mut body = column![header].spacing(10).width(Length::Fill);

    if let Some(warning) = &schema.warning {
        if !warning.is_empty() {
            body = body.push(warning_box(warning, colors));
        }
    }

    body = body.push(
        row![
            version_selector(
                schema.id.clone(),
                versions,
                version,
                colors,
                Message::SchemaVersionSelected
            ),
            container(text("")).width(Length::Fill),
            schema_action(
                schema.id.clone(),
                installed,
                downloaded,
                downloading,
                installing,
                progress,
                colors,
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill),
    );

    container(body)
        .width(Length::Fill)
        .padding(14)
        .style(move |_| card_style(colors))
        .into()
}

fn schema_action<'a>(
    schema_id: String,
    installed: bool,
    downloaded: bool,
    downloading: bool,
    installing: bool,
    progress: Option<f32>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    if installing {
        button_disabled("安装中…", &colors)
    } else if downloading {
        let label = match progress {
            Some(p) => format!("下载中 {:.0}%", p * 100.0),
            None => "下载中…".to_string(),
        };
        button_disabled(label, &colors)
    } else if installed {
        // 已安装的方案必须能从商店页卸载：只显示「已安装」纯文本时，
        // 用户想换方案只能去「输入方案」页找入口（U-31 同类问题）。
        button_danger("卸载", &colors, Message::UninstallSchema(schema_id)).into()
    } else if downloaded {
        // 已下载但未安装：给出「安装」入口，否则用户只能再去「输入方案」页。
        button_primary("安装", &colors, Message::InstallSchema(schema_id)).into()
    } else {
        button_primary("下载", &colors, Message::DownloadSchema(schema_id)).into()
    }
}

// ---- 模型 Tab ----

fn models_tab<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let store = &settings.market_model;

    let content: Element<'a, Message> = if store.loading && !store.loaded {
        center_msg(colors, "正在加载模型列表…")
    } else if let Some(error) = &store.error {
        error_view(colors, error)
    } else if store.loaded {
        model_list(store, colors)
    } else {
        center_msg(colors, "正在加载模型列表…")
    };

    let mut column = column![content].spacing(10).width(Length::Fill);

    if let Some(msg) = &store.install_message {
        column = column.push(text(msg.clone()).size(13).color(colors.error));
    }
    if store.loaded {
        column = column.push(footer(
            store.models.len(),
            "模型",
            &store.updated_at,
            colors,
        ));
    }

    column.into()
}

fn model_list<'a>(store: &'a MarketModelState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let tags: Vec<String> = MODEL_CATEGORIES
        .iter()
        .filter(|(key, _)| store.models.iter().any(|m| m.category == *key))
        .map(|(_, label)| label.to_string())
        .collect();

    let models: Vec<&MarketModel> = store
        .models
        .iter()
        .filter(|m| {
            store
                .selected_tag
                .as_ref()
                .map(|t| model_category_label(&m.category) == *t)
                .unwrap_or(true)
        })
        .collect();

    if models.is_empty() {
        return center_msg(colors, "没有匹配的模型");
    }

    let mut list = column![].spacing(8).width(Length::Fill);
    if !tags.is_empty() {
        list = list.push(chip_bar(tags, &store.selected_tag, colors));
    }
    list = list.push(grid_rows(models.iter().map(|model| {
        let is_downloaded = store.downloaded_ids.contains(&model.id);
        let is_downloading = store.downloading.as_deref() == Some(model.id.as_str());
        let progress = if is_downloading {
            store.download_progress
        } else {
            None
        };
        model_card(
            model,
            is_downloaded,
            is_downloading,
            progress,
            store.selected_versions.get(&model.id).cloned(),
            colors,
        )
    })));
    list.into()
}

fn model_card<'a>(
    model: &'a MarketModel,
    downloaded: bool,
    downloading: bool,
    progress: Option<f32>,
    selected_version: Option<String>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let versions: Vec<String> = model.versions.iter().map(|v| v.version.clone()).collect();
    let version = selected_version
        .or_else(|| model.current_version.clone())
        .unwrap_or_else(|| versions.first().cloned().unwrap_or_default());

    let size_label = model
        .versions
        .iter()
        .find(|v| v.version == version)
        .or_else(|| model.versions.first())
        .map(|v| v.size.as_deref().map(format_size).unwrap_or_default())
        .or_else(|| {
            if model.size.is_empty() {
                None
            } else {
                Some(format_size(&model.size))
            }
        })
        .unwrap_or_default();

    let category_label = model_category_label(&model.category);
    let glyph = category_label
        .chars()
        .next()
        .or_else(|| model.name.chars().next())
        .unwrap_or('模');

    let tags = if category_label.is_empty() {
        Vec::new()
    } else {
        vec![category_label]
    };

    let mut body = column![row![
        glyph_box(glyph, model_icon_color(&model.category, colors)),
        column![
            row![
                text(if model.name.is_empty() {
                    model.id.clone()
                } else {
                    model.name.clone()
                })
                .size(15)
                .font(semibold())
                .color(colors.foreground),
                container(text("")).width(Length::Fill),
                if size_label.is_empty() {
                    text("").size(13)
                } else {
                    text(size_label).size(13).color(colors.foreground_muted)
                },
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            if model.author.is_empty() && tags.is_empty() {
                iced::widget::Space::new().height(12).into()
            } else {
                author_tags_row(&model.author, &tags, colors)
            },
            if model.description.is_empty() {
                text("").size(13)
            } else {
                text(truncate(&model.description, DESC_MAX_CHARS))
                    .size(13)
                    .color(colors.foreground_muted)
            },
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
    .width(Length::Fill),]
    .spacing(10)
    .width(Length::Fill);

    body = body.push(
        row![
            version_selector(
                model.id.clone(),
                versions,
                version,
                colors,
                Message::ModelVersionSelected
            ),
            container(text("")).width(Length::Fill),
            model_action(model.id.clone(), downloaded, downloading, progress, colors,),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill),
    );

    container(body)
        .width(Length::Fill)
        .padding(14)
        .style(move |_| card_style(colors))
        .into()
}

fn model_action<'a>(
    model_id: String,
    downloaded: bool,
    downloading: bool,
    progress: Option<f32>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    if downloading {
        let label = match progress {
            Some(p) => format!("下载中 {:.0}%", p * 100.0),
            None => "下载中…".to_string(),
        };
        button_disabled(label, &colors)
    } else if downloaded {
        row![
            text("已安装").size(13).color(colors.foreground_muted),
            button_danger("删除", &colors, Message::DeleteModel(model_id)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    } else {
        button_primary("获取", &colors, Message::DownloadModel(model_id)).into()
    }
}

fn model_category_label(category: &str) -> String {
    MODEL_CATEGORIES
        .iter()
        .find(|(key, _)| *key == category)
        .map(|(_, label)| label.to_string())
        .unwrap_or_default()
}

// ---- 插件 Tab ----

fn plugins_tab<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let store = &settings.market_plugin;

    let content: Element<'a, Message> = if store.loading && !store.loaded {
        center_msg(colors, "正在加载插件列表…")
    } else if let Some(error) = &store.error {
        error_view(colors, error)
    } else if store.loaded {
        plugin_list(store, colors)
    } else {
        center_msg(colors, "正在加载插件列表…")
    };

    let mut column = column![content].spacing(10).width(Length::Fill);

    if let Some(msg) = &store.install_message {
        column = column.push(text(msg.clone()).size(13).color(colors.error));
    }
    if store.loaded {
        column = column.push(footer(
            store.plugins.len(),
            "插件",
            &store.updated_at,
            colors,
        ));
    }

    column.into()
}

fn plugin_list<'a>(store: &'a MarketPluginState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let tags: Vec<String> = PLUGIN_CATEGORIES
        .iter()
        .filter(|(key, _)| store.plugins.iter().any(|p| plugin_kind(p) == *key))
        .map(|(_, label)| label.to_string())
        .collect();

    let plugins: Vec<&MarketPlugin> = store
        .plugins
        .iter()
        .filter(|p| {
            store
                .selected_tag
                .as_ref()
                .map(|t| plugin_category_label(plugin_kind(p)) == *t)
                .unwrap_or(true)
        })
        .collect();

    if plugins.is_empty() {
        return center_msg(colors, "没有匹配的插件");
    }

    let mut list = column![].spacing(8).width(Length::Fill);
    if !tags.is_empty() {
        list = list.push(chip_bar(tags, &store.selected_tag, colors));
    }
    list = list.push(grid_rows(plugins.iter().map(|plugin| {
        let installed = store.installed.iter().find(|r| r.id == plugin.id).cloned();
        let is_downloading = store.downloading.as_deref() == Some(plugin.id.as_str());
        let is_installing = store.installing.as_deref() == Some(plugin.id.as_str());
        let progress = if is_downloading {
            store.download_progress
        } else {
            None
        };
        let downloaded = store.downloaded_ids.contains(&plugin.id);
        plugin_card(
            plugin,
            installed,
            downloaded,
            is_downloading,
            is_installing,
            progress,
            colors,
        )
    })));
    list.into()
}

/// 插件类型：优先 pluginType 字段（新索引），兼容旧 type 字段。
fn plugin_kind(plugin: &MarketPlugin) -> &str {
    let kind = plugin.plugin_kind.trim();
    if !kind.is_empty() {
        kind
    } else {
        &plugin.plugin_type
    }
}

fn plugin_category_label(kind: &str) -> String {
    PLUGIN_CATEGORIES
        .iter()
        .find(|(key, _)| *key == kind)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| kind.to_string())
}

fn plugin_card<'a>(
    plugin: &'a MarketPlugin,
    installed: Option<xime_plugin::PluginRecord>,
    downloaded: bool,
    downloading: bool,
    installing: bool,
    progress: Option<f32>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let version = plugin
        .current_version
        .clone()
        .or_else(|| plugin.versions.first().map(|v| v.version.clone()))
        .unwrap_or_default();

    let kind = plugin_kind(plugin);
    let kind_label = plugin_category_label(kind);
    let glyph = kind_label.chars().next().unwrap_or('插');

    let tags = if kind_label.is_empty() {
        Vec::new()
    } else {
        vec![kind_label]
    };

    let installed_version = installed
        .as_ref()
        .map(|r| r.version.clone())
        .unwrap_or_default();
    let is_installed = installed.is_some();
    let enabled = installed.as_ref().map(|r| r.enabled).unwrap_or(false);

    let mut body = column![row![
        glyph_box(glyph, plugin_icon_color(kind, colors)),
        column![
            row![text(if plugin.name.is_empty() {
                plugin.id.clone()
            } else {
                plugin.name.clone()
            })
            .size(15)
            .font(semibold())
            .color(colors.foreground),]
            .spacing(6)
            .align_y(Alignment::Center),
            if plugin.author.is_empty() && tags.is_empty() {
                iced::widget::Space::new().height(12).into()
            } else {
                author_tags_row(&plugin.author, &tags, colors)
            },
            if plugin.description.is_empty() {
                text("").size(13)
            } else {
                text(truncate(&plugin.description, DESC_MAX_CHARS))
                    .size(13)
                    .color(colors.foreground_muted)
            },
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
    .width(Length::Fill),]
    .spacing(10)
    .width(Length::Fill);

    let mut footer = row![
        if version.is_empty() {
            text("").size(13)
        } else {
            text(format!("v{version}")).size(13).color(colors.primary)
        },
        if is_installed {
            text(if enabled { "已启用" } else { "已禁用" })
                .size(13)
                .color(colors.foreground_muted)
        } else if !installed_version.is_empty() {
            text(format!("已安装 v{installed_version}"))
                .size(13)
                .color(colors.foreground_muted)
        } else {
            text("").size(13)
        },
        container(text("")).width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Length::Fill);

    if is_installed {
        footer = footer.push(switch(enabled, colors, move |on| {
            Message::TogglePlugin(plugin.id.clone(), on)
        }));
    }
    footer = footer.push(plugin_action(
        plugin.id.clone(),
        is_installed,
        downloaded,
        downloading,
        installing,
        progress,
        colors,
    ));

    body = body.push(footer);

    container(body)
        .width(Length::Fill)
        .padding(14)
        .style(move |_| card_style(colors))
        .into()
}

fn plugin_action<'a>(
    plugin_id: String,
    installed: bool,
    downloaded: bool,
    downloading: bool,
    installing: bool,
    progress: Option<f32>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    if installing {
        button_disabled("安装中…", &colors)
    } else if downloading {
        let label = match progress {
            Some(p) => format!("下载中 {:.0}%", p * 100.0),
            None => "下载中…".to_string(),
        };
        button_disabled(label, &colors)
    } else if installed {
        button_danger("卸载", &colors, Message::UninstallPlugin(plugin_id)).into()
    } else if downloaded {
        button_primary("安装", &colors, Message::InstallPlugin(plugin_id)).into()
    } else {
        button_primary("获取", &colors, Message::DownloadPlugin(plugin_id)).into()
    }
}

// ---- 公共小组件 ----

/// 卡片标题下方的「作者 + 标签」行。
fn author_tags_row<'a>(
    author: &str,
    tags: &[String],
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let mut row = row![].spacing(6).align_y(Alignment::Center);
    if !author.is_empty() {
        row = row.push(
            text(author.to_string())
                .size(13)
                .color(colors.foreground_muted),
        );
    }
    for tag in tags.iter().take(3) {
        row = row.push(pill(tag, colors));
    }
    row.into()
}

fn center_msg<'a>(colors: &'a ThemeColors, msg: &'a str) -> Element<'a, Message> {
    container(text(msg).size(14).color(colors.foreground_muted))
        .width(Length::Fill)
        .padding(32)
        .center_x(Length::Fill)
        .into()
}

fn error_view<'a>(colors: &'a ThemeColors, error: &'a str) -> Element<'a, Message> {
    column![
        text(error.to_string())
            .size(14)
            .color(colors.foreground_muted),
        text_button("重试", colors, Message::MarketRetry),
    ]
    .spacing(12)
    .align_x(Alignment::Start)
    .width(Length::Fill)
    .padding(24)
    .into()
}

fn footer<'a>(
    count: usize,
    kind: &str,
    updated_at: &'a str,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let mut text_ = format!("共 {} 个{}", count, kind);
    if !updated_at.is_empty() {
        text_ = format!("{} · 更新于 {}", text_, updated_at);
    }
    text(text_).size(12).color(colors.foreground_faint).into()
}

/// 彩色方块：分类字 + 分类色浅底（商店卡片图标位，参考 Xime 48dp 圆角 12dp）。
fn glyph_box<'a>(glyph: char, icon: (Color, Color)) -> Element<'a, Message> {
    let (container_bg, content_color) = icon;
    container(
        text(glyph.to_string())
            .size(14)
            .font(semibold())
            .color(content_color),
    )
    .width(28)
    .height(28)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_| container::Style {
        background: Some(Background::Color(container_bg)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(RADIUS_MD),
        },
        ..container::Style::default()
    })
    .into()
}

/// 方案图标色：统一用主色容器。
fn schema_icon_color(colors: &ThemeColors) -> (Color, Color) {
    (colors.primary_dim, colors.primary)
}

/// 模型图标色：按分类区分（参照 Xime modelCategoryColors）。
fn model_icon_color(category: &str, colors: &ThemeColors) -> (Color, Color) {
    match category {
        "prediction" => (colors.primary_dim, colors.primary),
        "handwriting" => (colors.secondary_dim, colors.secondary),
        "asr" => (colors.tertiary_dim, colors.tertiary),
        _ => (colors.surface_variant, colors.foreground_muted),
    }
}

/// 插件图标色：按类型区分（参照 Xime pluginCategoryColors）。
fn plugin_icon_color(kind: &str, colors: &ThemeColors) -> (Color, Color) {
    match kind {
        "emoji" => (colors.tertiary_dim, colors.tertiary),
        "speech" => (colors.primary_dim, colors.primary),
        "prediction" => (colors.secondary_dim, colors.secondary),
        _ => (colors.surface_variant, colors.foreground_muted),
    }
}

/// 卡片描述最多显示的字符数（3 列卡片宽度下约 2 行中文）。
const DESC_MAX_CHARS: usize = 36;

/// 截断长文本（描述等），超出 `max_chars` 加省略号。
fn truncate(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

/// 版本选择：多版本用下拉，单版本直接展示。
fn version_selector<'a>(
    id: String,
    versions: Vec<String>,
    selected: String,
    colors: &'a ThemeColors,
    on_select: fn(String, String) -> Message,
) -> Element<'a, Message> {
    if versions.is_empty() {
        return text("").size(13).into();
    }
    if versions.len() <= 1 {
        return text(selected).size(13).color(colors.primary).into();
    }
    let colors = *colors;
    pick_list(versions, Some(selected), move |v| on_select(id.clone(), v)).text_size(14).padding([8, 10])
        .placeholder("版本")
        .padding([4, 8])
        .text_size(12)
        .width(Length::Shrink)
        .style(move |_theme, status| pick_list_style(&colors, status))
        .into()
}

fn pill<'a>(label: &str, colors: &'a ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    container(
        text(label.to_string())
            .size(12)
            .color(colors.foreground_muted),
    )
    .padding([2, 8])
    .style(move |_| container::Style {
        background: Some(Background::Color(colors.surface_variant)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(6.0),
        },
        ..container::Style::default()
    })
    .into()
}

fn warning_box<'a>(warning: &str, colors: &'a ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    container(text(warning.to_string()).size(13).color(colors.foreground))
        .width(Length::Fill)
        .padding(10)
        .style(move |_| container::Style {
            background: Some(Background::Color(colors.error_dim)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(6.0),
            },
            ..container::Style::default()
        })
        .into()
}

pub(crate) fn format_size(s: &str) -> String {
    let s = s.trim().to_lowercase();
    if s.ends_with("kb") || s.ends_with("mb") || s.ends_with("gb") {
        s.to_string()
    } else if let Ok(bytes) = s.parse::<u64>() {
        if bytes > 1024 * 1024 * 1024 {
            format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        } else if bytes > 1024 * 1024 {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        } else if bytes > 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{} B", bytes)
        }
    } else {
        s.to_string()
    }
}
