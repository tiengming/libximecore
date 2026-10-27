#[cfg(any(windows, feature = "dict-page"))]
use crate::components::widgets::text_input_style;
use crate::components::widgets::{
    badge, button_danger, button_disabled, button_primary, button_secondary, card_style, semibold,
    text_button,
};
use crate::state::{Message, SettingsState};
use crate::theme::ThemeColors;
use iced::widget::text_input;
use iced::widget::{button, column, container, row, text, Space};
use iced::{border, Alignment, Background, Border, Color, Element, Length};
use xime_config::schema_manifest::{package_label, BUILTIN_PACKAGE_ID};
use xime_config::SchemaInfo;

pub fn view<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let tab = settings.input_schema.current_tab;
    let installed_ids = settings.market_schema.installed_ids.clone();
    // 进行中的方案包操作（安装/卸载/还原）：哪个包在忙 + 是否有任务在跑。
    let busy_id = settings.market_schema.installing.as_deref();
    let any_busy =
        settings.market_schema.installing.is_some() || settings.market_schema.downloading.is_some();

    let header = row![
        text("输入方案")
            .size(20)
            .font(semibold())
            .color(colors.foreground),
        Space::new().width(Length::Fill),
        button_primary("部署方案", colors, Message::DeploySchemas),
        button_primary("打开数据目录", colors, Message::OpenUserDataDir),
        button_secondary("导入方案", colors, Message::SchemaImportOpen),
    ]
    .align_y(Alignment::Center)
    .spacing(12);

    let mut content = column![header, tab_bar(tab, colors)]
        .spacing(16)
        .padding(20)
        .width(Length::Fill);

    content = content.push(if tab == 0 {
        installed_tab(settings, colors)
    } else if tab == 1 {
        downloads_tab(
            &installed_ids,
            busy_id,
            any_busy,
            settings.market_schema.install_message.as_deref(),
            colors,
        )
    } else {
        schema_dict_tab(settings, colors)
    });

    content.into()
}

/// 按钮展示态：标签 + 是否可点 + 是否危险色（卸载类）。
///
/// 「安装中…」「卸载中…」这类进行中标签由 `action_button` 统一决定，
/// 页面只负责把「本卡片在忙 / 其它任务在忙」两个事实喂进来。
#[derive(Debug, PartialEq, Eq)]
struct ActionButton {
    label: &'static str,
    enabled: bool,
    danger: bool,
}

/// 纯函数：决定卡片按钮长什么样（便于单测，不依赖 iced）。
///
/// - `busy_self`：就是这张卡片对应的包在忙 → 换成进行中文案并禁用；
/// - `any_busy`：别的包/下载任务在忙 → 保持原标签但禁用（避免连点）。
fn action_button(
    idle_label: &'static str,
    busy_label: &'static str,
    danger: bool,
    busy_self: bool,
    any_busy: bool,
) -> ActionButton {
    if busy_self {
        ActionButton {
            label: busy_label,
            enabled: false,
            danger,
        }
    } else {
        ActionButton {
            label: idle_label,
            enabled: !any_busy,
            danger,
        }
    }
}

/// 安装冲突确认弹窗（对齐安卓「文件冲突：需要先卸载冲突方案：X，是否继续？」）。
///
/// 在 app.rs 里全局渲染：输入方案页与扩展商店页的安装入口共用同一确认流程，
/// 未确认前不动任何文件。
pub fn conflict_dialog<'a>(
    settings: &'a SettingsState,
    colors: &'a ThemeColors,
) -> Option<Element<'a, Message>> {
    let conflict = settings.market_schema.conflict_install.as_ref()?;
    let colors = *colors;
    let packages = conflict
        .packages
        .iter()
        .map(|pkg| package_label(pkg))
        .collect::<Vec<_>>()
        .join("、");
    let restore = conflict.is_restore_builtin();
    let action_label = if restore {
        "还原「内置方案包」"
    } else {
        "安装"
    };
    let confirm_label = if restore {
        "确认卸载并还原"
    } else {
        "确认卸载并安装"
    };

    let dialog = column![
        text("文件冲突")
            .size(16)
            .font(semibold())
            .color(colors.foreground),
        text(format!(
            "为避免多个方案来源混装在 rime 目录，{}前需要先卸载本机已有方案包：",
            if restore {
                format!("还原「{}」", package_label(BUILTIN_PACKAGE_ID))
            } else {
                format!("安装「{}」", conflict.schema_id)
            }
        ))
        .size(13)
        .color(colors.foreground_muted),
        text(packages).size(13).color(colors.foreground),
        text(
            "方案隔离：同一时刻 rime 目录里只保留一个方案来源。\
              卸载会删掉该方案包的文件连同它生成的配置（`<方案>.custom.yaml`、\
              短语表、部署缓存）；输入记录（*.userdb）与已下载的安装包保留；\
              内置方案包已自动备份（market/builtin/），可随时还原。"
        )
        .size(12)
        .color(colors.foreground_muted),
        text(format!("目标：{}", action_label))
            .size(12)
            .color(colors.foreground_muted),
        row![
            Space::new().width(Length::Fill),
            text_button("取消", &colors, Message::CancelSchemaInstall),
            button_primary(confirm_label, &colors, Message::ConfirmSchemaInstall),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Length::Fill);

    Some(dialog.into())
}

/// 「导入本地方案」弹窗：拖放 / 本地路径 / 远程 URL / 剪贴板。
pub fn import_dialog<'a>(
    settings: &'a SettingsState,
    colors: &'a ThemeColors,
) -> Option<Element<'a, Message>> {
    if !settings.schema_import.open {
        return None;
    }
    let state = &settings.schema_import;
    let colors = *colors;
    let busy = state.busy;
    let status = state
        .message
        .clone()
        .unwrap_or_else(|| "支持 .zip / .tar.gz；拖入文件或填写一种来源".to_string());

    let dialog = column![
        text("导入方案")
            .size(16)
            .font(semibold())
            .color(colors.foreground),
        text("把方案包拖到窗口里，或填本地路径 / 远程 URL")
            .size(13)
            .color(colors.foreground_muted),
        container(
            text("拖入 .zip / .tar.gz 方案包")
                .size(14)
                .color(colors.foreground_muted)
        )
        .width(Length::Fill)
        .padding(18)
        .center_x(Length::Fill)
        .style(move |_| card_style(&colors)),
        text("本地文件").size(12).color(colors.foreground_muted),
        row![
            text_input("C:\\path\\to\\schema.zip", &state.path)
                .on_input(Message::SchemaImportPathChanged)
                .style(move |_t, s| text_input_style(&colors, s))
                .width(Length::Fill),
            if state.browsing {
                button_disabled("浏览中…", &colors)
            } else {
                button_secondary("浏览", &colors, Message::SchemaImportBrowse).into()
            },
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text("远程 URL").size(12).color(colors.foreground_muted),
        text_input("https://example.com/schema.zip", &state.url)
            .on_input(Message::SchemaImportUrlChanged)
            .style(move |_t, s| text_input_style(&colors, s))
            .width(Length::Fill),
        row![
            text_button("从剪贴板读取文件 / URL", &colors, Message::SchemaImportClipboard),
            Space::new().width(Length::Fill),
        ]
        .spacing(6),
        text("方案名称").size(12).color(colors.foreground_muted),
        text_input("显示给用户的名称（可改）", &state.package_id)
            .on_input(Message::SchemaImportPackageIdChanged)
            .style(move |_t, s| text_input_style(&colors, s))
            .width(Length::Fill),
        {
            let (label, progress_value) = match state.progress {
                Some((done, total, bps)) if total > 0 => {
                    let pct = (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
                    (
                        format!(
                            "{pct:.0}% · {} / {} · {}/s",
                            format_bytes(done),
                            format_bytes(total),
                            format_bytes(bps)
                        ),
                        Some(done as f64 / total as f64),
                    )
                }
                Some((done, 0, bps)) => (
                    format!("已接收 {} · {}/s", format_bytes(done), format_bytes(bps)),
                    None,
                ),
                Some(_) => (String::new(), None),
                None if busy => ("正在准备…".to_string(), None),
                None => (String::new(), None),
            };
            container(
                column![
                    progress_track(progress_value, state.tick, colors),
                    text(label).size(12).color(colors.foreground_muted),
                ]
                .spacing(4)
                .height(Length::Fixed(34.0)),
            )
        },
        text(status).size(12).color(if state.message.is_some() {
            colors.error
        } else {
            colors.foreground_muted
        }),
        row![
            Space::new().width(Length::Fill),
            text_button("取消", &colors, Message::SchemaImportClose),
            if busy {
                button_disabled("导入中…", &colors)
            } else {
                button_primary("导入并安装", &colors, Message::SchemaImportStart).into()
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Length::Fill);

    Some(dialog.into())
}

/// 分段标签：已安装 / 已下载 / 方案词表。
fn tab_bar<'a>(active: usize, colors: &'a ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    // 「方案词表」tab 仅在启用词典机制时存在（Windows 恒开；Linux 走 dict-page）。
    #[cfg(any(windows, feature = "dict-page"))]
    let labels = ["已安装", "已下载", "方案词表"];
    #[cfg(not(any(windows, feature = "dict-page")))]
    let labels = ["已安装", "已下载"];
    let mut bar = row![].spacing(2).padding(2);

    for (i, label) in labels.iter().enumerate() {
        let is_active = i == active;
        let label = *label;
        bar = bar.push(
            button(text(label).size(14).color(if is_active {
                colors.primary
            } else {
                colors.foreground_muted
            }))
            .padding([7, 16])
            .style(move |_style, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if is_active || hovered {
                        Some(Background::Color(colors.surface))
                    } else {
                        None
                    },
                    text_color: if is_active {
                        colors.primary
                    } else {
                        colors.foreground_muted
                    },
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: border::radius(8.0),
                    },
                    ..button::Style::default()
                }
            })
            .on_press(Message::SchemaTab(i)),
        );
    }

    container(bar)
        .width(Length::Shrink)
        .style(move |_| container::Style {
            background: Some(Background::Color(colors.surface_variant)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// 已安装方案列表 + 部署。
///
/// 按方案包分组展示（内置方案包在前）：列表事实源仍是 rime 目录里的
/// `*.schema.yaml`，归属来自数据根 `.registry.yaml` 的包清单——第三方方案与
/// 内置方案因此各自成组，不再混在一起。
fn installed_tab<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    let schemas = settings.input_schema.available_schemas.clone();
    let packages = settings.input_schema.schema_packages.clone();
    let deployed = settings.input_schema.deployed_schema_ids.clone();
    let selected = settings.input_schema.selected_schema;
    let busy_id = settings.market_schema.installing.as_deref();
    let any_busy =
        settings.market_schema.installing.is_some() || settings.market_schema.downloading.is_some();

    if schemas.is_empty() {
        return container(
            text("暂无已安装的方案")
                .size(14)
                .color(colors.foreground_muted),
        )
        .width(Length::Fill)
        .padding(24)
        .center_x(Length::Fill)
        .into();
    }

    // 分组成 [包 id → 方案索引]；内置方案包优先，其余按包 id 排序。
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for i in 0..schemas.len() {
        let pkg = packages
            .get(i)
            .cloned()
            .unwrap_or_else(|| BUILTIN_PACKAGE_ID.to_string());
        match groups.iter_mut().find(|(id, _)| *id == pkg) {
            Some((_, indexes)) => indexes.push(i),
            None => groups.push((pkg, vec![i])),
        }
    }
    groups.sort_by_key(|g| group_order(&g.0));

    let mut list = column![].spacing(6).width(Length::Fill);
    for (pkg, indexes) in &groups {
        list = list.push(group_header(pkg, indexes.len(), colors));
        for i in indexes {
            let Some(schema) = schemas.get(*i) else {
                continue;
            };
            let is_deployed = deployed.iter().any(|id| id == &schema.schema_id);
            list = list.push(schema_row(
                schema,
                *i,
                *i == selected,
                pkg,
                is_deployed,
                colors,
            ));
        }
    }

    // 内置方案包被卸载（隔离安装第三方方案）后可从备份一键还原。
    if groups.iter().any(|(pkg, _)| pkg == BUILTIN_PACKAGE_ID) {
        let cleaning = busy_id == Some(BUILTIN_PACKAGE_ID);
        list = list.push(clean_card(
            cleaning,
            any_busy,
            settings.market_schema.builtin_clean_confirm,
            colors,
        ));
    }
    if settings.input_schema.builtin_restorable {
        let restoring = busy_id == Some(BUILTIN_PACKAGE_ID);
        list = list.push(restore_card(restoring, any_busy, colors));
    }

    if let Some(msg) = &settings.market_schema.install_message {
        list = list.push(text(msg.clone()).size(13).color(colors.error));
    }

    list.into()
}

/// 分组排序键：内置方案包在前，其余按包 id 升序。
fn group_order(package_id: &str) -> (u8, String) {
    if package_id == BUILTIN_PACKAGE_ID {
        (0, String::new())
    } else if package_id == xime_config::schema_manifest::LOCAL_PACKAGE_ID {
        (1, String::new())
    } else {
        (2, package_id.to_string())
    }
}

/// 方案包分组标题（来源标注：内置方案包 / <包 id>）。
fn group_header<'a>(package_id: &str, count: usize, colors: ThemeColors) -> Element<'a, Message> {
    let is_builtin = package_id == BUILTIN_PACKAGE_ID;
    let is_local = package_id == xime_config::schema_manifest::LOCAL_PACKAGE_ID;
    let label = xime_config::schema_manifest::package_label(package_id);
    let mut head = row![
        text(label)
            .size(13)
            .font(semibold())
            .color(colors.foreground_muted),
        badge(
            if is_builtin {
                "内置"
            } else if is_local {
                "本地"
            } else {
                "第三方"
            },
            colors.foreground_muted,
            colors.surface_variant,
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    head = head.push(
        text(format!("{count} 个方案"))
            .size(12)
            .color(colors.foreground_muted),
    );

    container(head).width(Length::Fill).padding([8, 4]).into()
}

/// 单个已安装方案行。
///
/// `is_deployed`：`build/` 里是否有该方案的编译产物。没有就标「未启用」——
/// 不是不能点，而是点了需要先部署（见 `save_schema` 的兜底路径）。
fn schema_row<'a>(
    schema: &SchemaInfo,
    index: usize,
    is_current: bool,
    package_id: &str,
    is_deployed: bool,
    colors: ThemeColors,
) -> Element<'a, Message> {
    let display = if schema.name.is_empty() {
        schema.schema_id.clone()
    } else {
        format!("{} ({})", schema.name, schema.schema_id)
    };

    let mut left = row![text(display).size(14).color(if is_current {
        colors.on_primary
    } else {
        colors.foreground
    }),]
    .spacing(8)
    .align_y(Alignment::Center);
    if is_current {
        left = left.push(badge(
            "当前",
            colors.on_primary,
            Color::from_rgba(1.0, 1.0, 1.0, 0.2),
        ));
    }
    if !is_deployed {
        left = left.push(badge(
            "未启用",
            colors.foreground_muted,
            colors.surface_variant,
        ));
    }
    left = left.push(Space::new().width(Length::Fill));
    left = left.push(
        text(package_label(package_id))
            .size(12)
            .color(if is_current {
                colors.on_primary
            } else {
                colors.foreground_muted
            }),
    );
    left = left.push(text("设置").size(13).color(if is_current {
        colors.primary
    } else {
        colors.foreground_muted
    }));

    button(left)
        .on_press(Message::SelectSchema(index))
        .width(Length::Fill)
        .height(Length::Shrink)
        .padding([10, 12])
        .style(move |_style, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_current {
                    colors.primary
                } else if hovered {
                    colors.surface_hover
                } else {
                    colors.surface
                })),
                text_color: if is_current {
                    colors.on_primary
                } else {
                    colors.foreground
                },
                border: Border {
                    color: if is_current {
                        Color::TRANSPARENT
                    } else {
                        colors.border
                    },
                    width: 1.0,
                    radius: border::radius(8.0),
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// 内置方案包还原卡片（隔离切换到第三方方案后的退路，即「恢复默认方案」入口）。
fn restore_card<'a>(restoring: bool, any_busy: bool, colors: ThemeColors) -> Element<'a, Message> {
    let action = action_button("恢复默认方案", "还原中…", false, restoring, any_busy);
    let action_widget: Element<'a, Message> = if action.enabled {
        button_primary(action.label, &colors, Message::RestoreBuiltinSchema).into()
    } else {
        button_disabled(action.label, &colors)
    };
    let status = if restoring {
        "正在从 market/builtin/ 备份还原，完成后会自动部署…"
    } else {
        "从 market/builtin/ 备份恢复默认方案（若有第三方方案包，会先提示卸载）"
    };

    container(
        row![
            column![
                text("内置方案已卸载或不完整")
                    .size(14)
                    .color(colors.foreground),
                text(status).size(13).color(colors.foreground_muted),
            ]
            .spacing(2)
            .width(Length::Fill),
            action_widget,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .padding(12),
    )
    .width(Length::Fill)
    .style(move |_| card_style(&colors))
    .into()
}

fn clean_card<'a>(
    cleaning: bool,
    any_busy: bool,
    confirm: bool,
    colors: ThemeColors,
) -> Element<'a, Message> {
    let status = if cleaning {
        "正在删除内置方案文件并写「已清理」标记；重启不会自动注入…"
    } else if confirm {
        "确认清理？内置方案文件会被删除（备份保留，可随时恢复）"
    } else {
        "删除内置方案包文件（共享词典若被其他包声明会保留）；可从备份恢复"
    };
    let widgets: Vec<Element<'a, Message>> = if confirm {
        vec![
            text_button("取消", &colors, Message::CancelCleanBuiltin).into(),
            button_danger("确认清理", &colors, Message::ConfirmCleanBuiltin).into(),
        ]
    } else {
        let action = action_button("清理内置方案", "清理中…", true, cleaning, any_busy);
        let action_widget: Element<'a, Message> = if action.enabled {
            button_danger(action.label, &colors, Message::CleanBuiltinSchema).into()
        } else {
            button_disabled(action.label, &colors)
        };
        vec![action_widget]
    };
    let mut actions = row![].spacing(6).align_y(Alignment::Center);
    for widget in widgets {
        actions = actions.push(widget);
    }
    container(
        row![
            column![
                text("内置方案包").size(14).color(colors.foreground),
                text(status).size(13).color(colors.foreground_muted),
            ]
            .spacing(2)
            .width(Length::Fill),
            actions,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .padding(12),
    )
    .width(Length::Fill)
    .style(move |_| card_style(&colors))
    .into()
}

/// 已下载方案包列表。
///
/// `busy_id` 是当前正在安装/卸载的方案包 id，`any_busy` 表示有任务在跑：
/// 两者一起决定每张卡片的按钮是「安装 / 卸载」还是「安装中… / 卸载中…」。
fn downloads_tab<'a>(
    installed_ids: &[String],
    busy_id: Option<&str>,
    any_busy: bool,
    install_message: Option<&str>,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let pkg_ids = scan_market_dir();

    if pkg_ids.is_empty() {
        return column![
            text("暂无已下载的方案包")
                .size(14)
                .color(colors.foreground_muted),
            text("请前往「扩展商店」下载")
                .size(14)
                .color(colors.foreground_muted),
        ]
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .padding(48)
        .into();
    }

    let mut list = column![].spacing(8).width(Length::Fill);
    for pkg_id in &pkg_ids {
        let installed = installed_ids.contains(pkg_id);
        // 本机已有其他方案包 → 安装需先经冲突确认（方案隔离）。
        let needs_isolation = !installed && !installed_ids.is_empty();
        let busy_self = busy_id == Some(pkg_id.as_str());
        list = list.push(package_card(
            pkg_id,
            installed,
            needs_isolation,
            busy_self,
            any_busy,
            colors,
        ));
    }
    // 安装失败的提示也在这里显示：点「安装」的动作就在本页，错误不该只出现在另一个 tab。
    if let Some(msg) = install_message {
        list = list.push(text(msg.to_string()).size(13).color(colors.error));
    }
    list.into()
}

fn package_card<'a>(
    pkg_id: &str,
    installed: bool,
    needs_isolation: bool,
    busy_self: bool,
    any_busy: bool,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    let sid = pkg_id.to_string();
    let action = action_button(
        if installed { "卸载" } else { "安装" },
        if installed {
            "卸载中…"
        } else {
            "安装中…"
        },
        installed,
        busy_self,
        any_busy,
    );
    let action_widget: Element<'a, Message> = if action.enabled {
        if action.danger {
            button_danger(action.label, &colors, Message::UninstallSchema(sid.clone())).into()
        } else {
            button_primary(action.label, &colors, Message::InstallSchema(sid.clone())).into()
        }
    } else {
        button_disabled(action.label, &colors)
    };
    let delete_widget: Element<'a, Message> = if installed {
        button_disabled("先卸载", &colors)
    } else if busy_self || any_busy {
        button_disabled("删除", &colors)
    } else {
        button_danger("删除", &colors, Message::DeleteSchema(sid.clone())).into()
    };

    let status = if busy_self {
        if installed {
            "正在卸载：删除该方案包的文件与部署缓存…"
        } else {
            "正在安装：解压方案包并全量部署（词典较多时需要一会儿）…"
        }
    } else if installed {
        "已安装至输入法"
    } else if needs_isolation {
        "已下载，未安装（安装前会提示先卸载本机已有方案包）"
    } else {
        "已下载，未安装"
    };

    container(
        row![
            column![
                text(pkg_id.to_string()).size(14).color(colors.foreground),
                text(status).size(13).color(colors.foreground_muted),
            ]
            .spacing(2)
            .width(Length::Fill),
            action_widget,
            delete_widget,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .padding(12),
    )
    .width(Length::Fill)
    .style(move |_| card_style(&colors))
    .into()
}

/// 扫描本地市场缓存目录中的已下载方案包（跳过隐藏目录与内置方案备份目录）。
fn scan_market_dir() -> Vec<String> {
    // 与扩展商店下载共用同一目录（state::market_dir，%APPDATA%\<name>\market\）。
    let market_dir = crate::state::market_dir();

    let mut pkg_ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&market_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == BUILTIN_PACKAGE_ID {
                continue;
            }
            if entry.path().is_dir() {
                let has_archive = std::fs::read_dir(entry.path())
                    .map(|mut e| {
                        e.any(|e| {
                            e.ok()
                                .and_then(|e| {
                                    e.file_name()
                                        .to_str()
                                        .map(|n| n.ends_with(".zip") || n.ends_with(".tar.gz"))
                                })
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                if has_archive {
                    pkg_ids.push(name_str.to_string());
                }
            }
        }
    }
    pkg_ids
}

/// 「方案词表」tab：只读浏览当前方案的码表词条（主表 + import_tables 合并）。
///
/// 数据由 server 解析（`winxime-server::schema_dict`），本页只做搜索/翻页；
/// 词条来源是方案包自带的 `.dict.yaml`，改不得——要改词条用词典页的
/// 用户词典（造词）或快捷短语（整句上屏）。
#[cfg(any(windows, feature = "dict-page"))]
fn schema_dict_tab<'a>(
    settings: &'a SettingsState,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    let dict = &settings.input_schema.dict;

    if settings.input_schema.available_schemas.is_empty() {
        return container(
            text("暂无已安装的方案，安装后可在这里查看方案的码表词条")
                .size(14)
                .color(colors.foreground_muted),
        )
        .width(Length::Fill)
        .padding(24)
        .center_x(Length::Fill)
        .into();
    }

    let schema_name = settings
        .selected_schema_info()
        .map(|(_, name)| name)
        .unwrap_or_else(|| "（未选择）".to_string());

    let mut list = column![].spacing(8).width(Length::Fill);

    // 码表信息卡：主表 + 实际读入的表 + 缺失警告。
    let tables_text = if dict.tables.is_empty() {
        "（尚未读取，点「重新读取」）".to_string()
    } else {
        dict.tables.join("、")
    };
    let mut info = column![
        text(format!("方案：{schema_name}"))
            .size(14)
            .color(colors.foreground),
        text(format!("主码表：{}", dict.dict_name))
            .size(13)
            .color(colors.foreground_muted),
        text(format!("读入码表：{tables_text}"))
            .size(13)
            .color(colors.foreground_muted),
    ]
    .spacing(2)
    .width(Length::Fill);
    if !dict.missing.is_empty() {
        info = info.push(
            text(format!(
                "缺失码表（方案声明了但文件不存在）：{}",
                dict.missing.join("、")
            ))
            .size(12)
            .color(colors.error),
        );
    }
    list = list.push(
        container(info)
            .width(Length::Fill)
            .style(move |_| card_style(&colors)),
    );

    // 搜索行：输入关键词 + 重新读取。
    list = list.push(
        row![
            text_input("搜索词条或编码…", &dict.query)
                .on_input(Message::SchemaDictQueryChanged)
                .style(move |_t, s| text_input_style(&colors, s))
                .width(Length::Fill),
            text_button("重新读取", &colors, Message::SchemaDictRefresh),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    // 状态行（加载/失败/统计）。
    let status_color = if dict.error.is_some() {
        colors.error
    } else {
        colors.foreground_muted
    };
    list = list.push(
        text(dict.status_text())
            .size(13)
            .color(status_color)
            .width(Length::Fill),
    );

    // 词条行（只读）：词居左，编码居右。
    for entry in dict.page_entries() {
        list = list.push(
            container(
                row![
                    text(entry.word.clone()).size(14).color(colors.foreground),
                    Space::new().width(Length::Fill),
                    text(if entry.code.is_empty() {
                        "（无编码）".to_string()
                    } else {
                        entry.code.clone()
                    })
                    .size(13)
                    .color(colors.foreground_muted),
                ]
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .padding([6, 12]),
            )
            .width(Length::Fill)
            .style(move |_| card_style(&colors)),
        );
    }

    if dict.loaded && dict.error.is_none() && dict.total == 0 {
        list = list.push(
            container(
                text("这本方案的码表里没有词条（或没有读到数据）")
                    .size(13)
                    .color(colors.foreground_muted),
            )
            .width(Length::Fill)
            .padding(16)
            .center_x(Length::Fill),
        );
    }

    // 分页（与词典页词条浏览一致：每页 50 条）。
    let pages = dict.page_count();
    if pages > 1 {
        list = list.push(
            row![
                text(format!("第 {} / {} 页", dict.page + 1, pages))
                    .size(13)
                    .color(colors.foreground_muted),
                Space::new().width(Length::Fill),
                text_button(
                    "上一页",
                    &colors,
                    Message::SchemaDictPage(dict.page.saturating_sub(1)),
                ),
                text_button(
                    "下一页",
                    &colors,
                    Message::SchemaDictPage(dict.page.saturating_add(1)),
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill),
        );
    }

    list.into()
}

/// 未启用词典机制的兜底：tab 栏只渲染两个标签，此分支实际不可达，仅为满足编译。
#[cfg(not(any(windows, feature = "dict-page")))]
fn schema_dict_tab<'a>(
    settings: &'a SettingsState,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    installed_tab(settings, colors)
}

fn progress_track(progress: Option<f64>, tick: u32, colors: ThemeColors) -> Element<'static, Message> {
    const TRACK_WIDTH: f32 = 520.0;
    const TRACK_HEIGHT: f32 = 6.0;
    let (segment, offset) = match progress {
        Some(ratio) => (ratio.clamp(0.0, 1.0) as f32, 0.0),
        None => {
            // 总大小未知时用 22% 宽的扫描块来回移动，至少让用户看到「在动」。
            let phase = (tick % 60) as f32 / 60.0;
            let travel = 1.0 - 0.22;
            (0.22, phase * travel)
        }
    };
    let filled = (TRACK_WIDTH * segment).max(1.0);
    let gap = TRACK_WIDTH * offset;
    let track = row![
        Space::new().width(Length::Fixed(gap)),
        container(Space::new().width(Length::Fixed(filled)).height(Length::Fixed(TRACK_HEIGHT)))
            .style(move |_| container::Style {
                background: Some(Background::Color(colors.primary)),
                border: Border {
                    radius: border::radius(999.0),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        Space::new().width(Length::Fill),
    ]
    .height(Length::Fixed(TRACK_HEIGHT));
    container(track)
        .width(Length::Fixed(TRACK_WIDTH))
        .height(Length::Fixed(TRACK_HEIGHT))
        .style(move |_| container::Style {
            background: Some(Background::Color(colors.surface_variant)),
            border: Border {
                radius: border::radius(999.0),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::{action_button, ActionButton};

    /// 空闲：按当前状态显示「安装 / 卸载」，可点，卸载是危险色。
    #[test]
    fn idle_shows_install_or_uninstall() {
        assert_eq!(
            action_button("安装", "安装中…", false, false, false),
            ActionButton {
                label: "安装",
                enabled: true,
                danger: false
            }
        );
        assert_eq!(
            action_button("卸载", "卸载中…", true, false, false),
            ActionButton {
                label: "卸载",
                enabled: true,
                danger: true
            }
        );
    }

    /// 本卡片在忙：换成进行中文案并禁用（在忙的卡片点不动）。
    #[test]
    fn busy_self_shows_progress_label() {
        assert_eq!(
            action_button("安装", "安装中…", false, true, false),
            ActionButton {
                label: "安装中…",
                enabled: false,
                danger: false
            }
        );
        // busy_self 优先于 any_busy：标签要说明是「这个包」在忙。
        assert_eq!(
            action_button("卸载", "卸载中…", true, true, true),
            ActionButton {
                label: "卸载中…",
                enabled: false,
                danger: true
            }
        );
    }

    /// 其它任务在忙：保留原标签但禁用，避免连点（state 层也会早退）。
    #[test]
    fn other_task_busy_keeps_label_but_disables() {
        assert_eq!(
            action_button("安装", "安装中…", false, false, true),
            ActionButton {
                label: "安装",
                enabled: false,
                danger: false
            }
        );
        assert_eq!(
            action_button("恢复默认方案", "还原中…", false, false, true),
            ActionButton {
                label: "恢复默认方案",
                enabled: false,
                danger: false
            }
        );
    }
}
