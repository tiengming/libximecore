pub mod about;
pub mod appearance;
pub mod dictionary;
pub mod hotkeys;
pub mod input_schema;
pub mod plugins;
pub mod store;

#[cfg(feature = "backup-page")]
pub mod backup;
#[cfg(feature = "clipboard-page")]
pub mod clipboard;
#[cfg(feature = "pair-page")]
pub mod pair;
#[cfg(feature = "smart-suggestion-page")]
pub mod smart_suggestion;
#[cfg(feature = "voice-page")]
pub mod voice;

use crate::components::widgets::{medium, nav_button_style, scroll_style, semibold, sidebar_style};
use crate::state::{Message, SettingsState};
use crate::theme::ThemeColors;
use iced::widget::{button, column, container, row, scrollable, svg, text, Space};
use iced::{Alignment, Element, Length};

/// 侧栏分组：对齐 Android 版设置页的分组语义
/// （方案与词库 / 外观与交互 / 扩展 / 智能 / 同步 / 关于）。
pub fn sidebar_groups() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let mut groups: Vec<(&'static str, Vec<(&'static str, &'static str)>)> = vec![
        (
            "方案与词库",
            vec![
                ("icons/keyboard.svg", "输入方案"),
                ("icons/word.svg", "词典"),
            ],
        ),
        (
            "外观与交互",
            vec![
                ("icons/palette.svg", "外观"),
                ("icons/command.svg", "快捷键"),
            ],
        ),
        (
            "扩展",
            vec![
                ("icons/store.svg", "扩展商店"),
                ("icons/extension.svg", "插件管理"),
            ],
        ),
    ];

    // 「智能」组：智能联想 + 语音转文本（各条目自门控）。
    #[cfg(any(feature = "smart-suggestion-page", feature = "voice-page"))]
    {
        // with_capacity 绕开 vec_init_then_push：每项各自 cfg 门控，无法写成 vec! 字面量
        let mut smart_items: Vec<(&'static str, &'static str)> = Vec::with_capacity(2);
        #[cfg(feature = "smart-suggestion-page")]
        smart_items.push(("icons/thinking.svg", "智能联想"));
        #[cfg(feature = "voice-page")]
        smart_items.push(("icons/mic.svg", "语音转文本"));
        groups.push(("智能", smart_items));
    }

    #[cfg(any(
        target_os = "linux",
        feature = "pair-page",
        feature = "clipboard-page",
        feature = "backup-page"
    ))]
    {
        #[allow(unused_mut)] // pair-page 单独启用时仍需 push
        let mut sync_items: Vec<(&'static str, &'static str)> = Vec::new();
        #[cfg(feature = "pair-page")]
        sync_items.push(("icons/sync.svg", "设备关联"));
        #[cfg(feature = "clipboard-page")]
        sync_items.push(("icons/clipboard.svg", "剪贴板"));
        #[cfg(feature = "backup-page")]
        sync_items.push(("icons/backup.svg", "同步与备份"));
        if !sync_items.is_empty() {
            groups.push(("同步与备份", sync_items));
        }
    }

    groups.push(("关于", vec![("icons/about.svg", "关于")]));
    groups
}

/// 扁平导航项（分组拍平，页序 = 分组顺序内逐项）。
pub fn sidebar_items() -> Vec<(&'static str, &'static str)> {
    sidebar_groups()
        .into_iter()
        .flat_map(|(_, items)| items)
        .collect()
}

/// 「语音转文本」页在侧栏里的扁平下标（没编进这个页面时 None）。
///
/// 用标题反查下标而不是写死数字：侧栏分组顺序是会变的（这页现在就在「输入」
/// 分组里），写死迟早错位，而且错位是静默的（轮询挂到别的页上）。
#[cfg(feature = "voice-page")]
pub fn voice_page_index() -> Option<usize> {
    sidebar_items()
        .iter()
        .position(|(_, title)| *title == "语音转文本")
}

/// 侧栏导航：品牌区 + 分组标题 + 各组导航项（点击仍发扁平下标）。
pub fn sidebar(current: usize, colors: &ThemeColors) -> Element<'static, Message> {
    let groups = sidebar_groups();
    let colors = *colors;

    let mut nav = column![brand(&colors), Space::new().height(20)]
        .spacing(2)
        .padding([20, 14]);

    let mut flat = 0usize;
    for (title, items) in &groups {
        nav = nav.push(
            container(
                text(*title)
                    .size(12)
                    .font(semibold())
                    .color(colors.foreground_faint),
            )
            .width(Length::Fill)
            .padding([0, 10]),
        );
        nav = nav.push(Space::new().height(4));
        for (icon_path, name) in items {
            nav = nav.push(nav_button(icon_path, name, flat, flat == current, &colors));
            flat += 1;
        }
        nav = nav.push(Space::new().height(12));
    }

    // 分组标题增加侧栏高度，窗口过矮时可滚动。
    let nav: Element<'static, Message> = scrollable(nav)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |theme, status| scroll_style(&colors, theme, status))
        .into();

    container(nav)
        .width(200)
        .height(Length::Fill)
        .style(move |_| sidebar_style(&colors))
        .into()
}

/// 品牌区：应用图标 + 名称 + 副标题。
fn brand(colors: &ThemeColors) -> Element<'static, Message> {
    let logo: Element<'static, Message> = match crate::Assets::get("icons/xime.svg") {
        Some(f) => svg(svg::Handle::from_memory(f.data))
            .width(28)
            .height(28)
            .into(),
        None => Space::new().width(28).into(),
    };
    row![
        logo,
        column![
            text(xime_config::app_metadata().display_name)
                .size(15)
                .font(semibold())
                .color(colors.foreground),
            text("输入法设置")
                .size(12)
                .font(medium())
                .color(colors.foreground_muted),
        ]
        .spacing(1),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

/// 导航项：图标 + 文字。
fn nav_button(
    icon_path: &'static str,
    label: &'static str,
    index: usize,
    active: bool,
    colors: &ThemeColors,
) -> Element<'static, Message> {
    let colors = *colors;
    let icon: Element<'static, Message> = match crate::Assets::get(icon_path) {
        Some(f) => svg(svg::Handle::from_memory(f.data))
            .width(16)
            .height(16)
            .into(),
        None => Space::new().width(16).into(),
    };
    button(
        row![
            icon,
            text(label).size(14).font(medium()).color(if active {
                colors.primary
            } else {
                colors.foreground_muted
            }),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(Message::PageSelected(index))
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_theme, status| nav_button_style(&colors, status, active))
    .into()
}

/// 当前页面内容。
pub fn page_content<'a>(
    settings: &'a SettingsState,
    page: usize,
    colors: &'a ThemeColors,
) -> Element<'a, Message> {
    let items = sidebar_items();
    let page = page.min(items.len().saturating_sub(1));

    match items[page].1 {
        "输入方案" => input_schema::view(settings, colors),
        "外观" => appearance::view(settings, colors),
        "快捷键" => hotkeys::view(settings, colors),
        "词典" => dictionary::view(settings, colors),
        "扩展商店" => store::view(settings, colors),
        "插件管理" => plugins::view(settings, colors),
        #[cfg(feature = "smart-suggestion-page")]
        "智能联想" => smart_suggestion::view(settings, colors),
        #[cfg(feature = "pair-page")]
        "设备关联" => pair::view(settings, colors),
        #[cfg(feature = "clipboard-page")]
        "剪贴板" => clipboard::view(settings, colors),
        #[cfg(feature = "backup-page")]
        "同步与备份" => backup::view(settings, colors),
        #[cfg(feature = "voice-page")]
        "语音转文本" => voice::view(settings, colors),
        _ => about::view(settings, colors),
    }
}

pub fn scrollable_content<'a>(
    content: impl Into<Element<'a, Message>>,
    colors: &ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    scrollable(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |theme, status| scroll_style(&colors, theme, status))
        .into()
}
