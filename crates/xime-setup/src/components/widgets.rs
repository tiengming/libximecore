use crate::theme::ThemeColors;
use iced::widget::{
    button, container, opaque, row, stack, text, toggler, toggler::Status as TogglerStatus,
    Button,
};
use iced::{border, Alignment, Background, Border, Color, Element, Length};

/// 圆角（Tailwind rounded-lg/md）。
pub const RADIUS_LG: f32 = 12.0;
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_SM: f32 = 8.0;

/// 应用 UI 字体：显式指定微软雅黑 UI（Windows 全版本自带，拉丁+中文覆盖完整）。
/// 不用通用族名（Sans Serif）：fontdb 在 Windows 上的通用族解析可能命中
/// 图标字体（Segoe Fluent Icons 等，也被归类为 sans-serif），ASCII 会渲染成
/// 符号乱码（中文不受影响——图标字体不含 CJK，回退雅黑）。
pub const UI_FONT: iced::font::Font = iced::font::Font::with_name("Microsoft YaHei UI");

pub fn semibold() -> iced::font::Font {
    iced::font::Font {
        weight: iced::font::Weight::Semibold,
        ..UI_FONT
    }
}

/// 中等字重：小字号正文在 1x 显示器上补偿缺失的笔画加粗（CoreText
/// stem darkening），避免发虚。
pub fn medium() -> iced::font::Font {
    iced::font::Font {
        weight: iced::font::Weight::Medium,
        ..UI_FONT
    }
}

/// 居中模态弹窗：半透明遮罩 + 居中内容卡片（iced 0.14 无内置 Modal，stack 实现）。
/// `dialog` 为 None 时原样返回底页。
pub fn modal_dialog<'a, Message: 'a>(
    base: Element<'a, Message>,
    dialog: Option<Element<'a, Message>>,
    colors: &ThemeColors,
) -> Element<'a, Message> {
    modal_dialog_sized(base, dialog, colors, 520.0, None)
}

/// 固定尺寸版本：导入弹窗内容多，避免随着状态文字/进度行变化而改变窗口大小。
pub fn modal_dialog_sized<'a, Message: 'a>(
    base: Element<'a, Message>,
    dialog: Option<Element<'a, Message>>,
    colors: &ThemeColors,
    width: f32,
    height: Option<f32>,
) -> Element<'a, Message> {
    let Some(dialog) = dialog else {
        return base;
    };
    let colors = *colors;
    let mut card = container(dialog)
        .width(Length::Fixed(width))
        .max_width(width)
        .padding(16)
        .style(move |_| container::Style {
            background: Some(Background::Color(colors.surface)),
            text_color: Some(colors.foreground),
            border: Border {
                color: colors.border,
                width: 1.0,
                radius: border::radius(RADIUS_MD),
            },
            ..container::Style::default()
        });
    if let Some(height) = height {
        card = card.height(Length::Fixed(height));
    }
    // opaque：遮罩必须吃掉鼠标事件，否则点击会穿透到下面的页面。
    let overlay = opaque(
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::BLACK.scale_alpha(0.4))),
                ..container::Style::default()
            }),
    );
    stack![base, overlay].into()
}

// ---- container styles ----

/// 卡片容器样式：圆角 + 边框 + 柔和底色。
pub fn card_style(colors: &ThemeColors) -> container::Style {
    container::Style {
        background: Some(Background::Color(colors.surface)),
        text_color: Some(colors.foreground),
        border: Border {
            color: colors.border,
            width: 1.0,
            radius: border::radius(RADIUS_LG),
        },
        ..container::Style::default()
    }
}

/// 侧栏样式：无圆角的整块面板，与窗口底色分层。
pub fn sidebar_style(colors: &ThemeColors) -> container::Style {
    container::Style {
        background: Some(Background::Color(colors.surface)),
        text_color: Some(colors.foreground),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(0.0),
        },
        ..container::Style::default()
    }
}

/// 滚动条样式：窄滚动条 + 半透明圆角滑块，贴合深色主题。
pub fn scroll_style(
    colors: &ThemeColors,
    theme: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    let mut style = iced::widget::scrollable::default(theme, status);
    let rail = iced::widget::scrollable::Rail {
        background: None,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(999.0),
        },
        scroller: iced::widget::scrollable::Scroller {
            background: Background::Color(Color {
                a: 0.35,
                ..colors.primary
            }),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(999.0),
            },
        },
    };
    style.vertical_rail = rail;
    style.horizontal_rail = rail;
    style
}

// ---- button styles ----

/// 主要操作按钮：主色填充 + 主色文字。
pub fn primary_button_style(colors: &ThemeColors, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => colors.primary_hover,
        _ => colors.primary,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: colors.on_primary,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(RADIUS_MD),
        },
        ..button::Style::default()
    }
}

/// 危险按钮（卸载等）：错误色填充 + 白色文字。
pub fn danger_button_style(colors: &ThemeColors, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => colors.error,
        _ => colors.error,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: colors.on_error,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(RADIUS_MD),
        },
        ..button::Style::default()
    }
}

/// 次要操作按钮：透明底 + 边框，悬停浅色底。
pub fn secondary_button_style(colors: &ThemeColors, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if hovered {
            Some(Background::Color(colors.surface_hover))
        } else {
            None
        },
        text_color: colors.foreground,
        border: Border {
            color: colors.border_strong,
            width: 1.0,
            radius: border::radius(RADIUS_MD),
        },
        ..button::Style::default()
    }
}

/// 导航按钮样式：active 为主色浅底 + 主色文字；悬停为浅色底。
pub fn nav_button_style(
    colors: &ThemeColors,
    status: button::Status,
    active: bool,
) -> button::Style {
    let base = button::Style {
        background: None,
        text_color: if active {
            colors.primary
        } else {
            colors.foreground_muted
        },
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(RADIUS_MD),
        },
        ..button::Style::default()
    };
    if active {
        button::Style {
            background: Some(Background::Color(colors.primary_dim)),
            text_color: colors.primary,
            ..base
        }
    } else {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => {
                Some(Background::Color(colors.surface_hover))
            }
            _ => None,
        };
        button::Style {
            background: bg,
            ..base
        }
    }
}

/// 数字输入步进按钮：透明底，悬停浅色底。
pub fn number_button_style(colors: &ThemeColors, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => {
                Some(Background::Color(colors.surface_hover))
            }
            _ => None,
        },
        text_color: colors.foreground,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: border::radius(RADIUS_SM),
        },
        ..button::Style::default()
    }
}

/// 文本输入框样式：底色 + 边框，聚焦时主色描边。
pub fn text_input_style(
    colors: &ThemeColors,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let border_color = match status {
        iced::widget::text_input::Status::Focused { .. } => colors.primary,
        iced::widget::text_input::Status::Hovered => colors.border_strong,
        _ => colors.border,
    };
    iced::widget::text_input::Style {
        background: Background::Color(colors.surface),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: border::radius(RADIUS_MD),
        },
        icon: colors.foreground_muted,
        placeholder: colors.foreground_faint,
        value: colors.foreground,
        selection: colors.selection,
    }
}

// ---- buttons ----

/// 主要按钮：主色填充。
pub fn button_primary<'a, Message: Clone>(
    label: impl Into<String>,
    colors: &ThemeColors,
    on_press: Message,
) -> Button<'a, Message> {
    let colors = *colors;
    button(
        text(label.into())
            .size(14)
            .font(medium())
            .color(colors.on_primary),
    )
    .padding([8, 18])
    .style(move |_theme, status| primary_button_style(&colors, status))
    .on_press(on_press)
}

/// 次要按钮：透明底 + 边框。
pub fn button_secondary<'a, Message: Clone>(
    label: impl Into<String>,
    colors: &ThemeColors,
    on_press: Message,
) -> Button<'a, Message> {
    let colors = *colors;
    button(
        text(label.into())
            .size(14)
            .font(medium())
            .color(colors.foreground),
    )
    .padding([7, 14])
    .style(move |_theme, status| secondary_button_style(&colors, status))
    .on_press(on_press)
}

/// 危险按钮：错误色填充（卸载等破坏性操作）。
pub fn button_danger<'a, Message: Clone>(
    label: impl Into<String>,
    colors: &ThemeColors,
    on_press: Message,
) -> Button<'a, Message> {
    let colors = *colors;
    button(
        text(label.into())
            .size(14)
            .font(medium())
            .color(colors.on_error),
    )
    .padding([6, 14])
    .style(move |_theme, status| danger_button_style(&colors, status))
    .on_press(on_press)
}

/// 禁用态按钮：进行中的操作（「安装中…」「下载中…」「卸载中…」）占位用。
/// 尺寸与次要按钮一致，只把文字压成弱化色，且不带 `on_press`（点不动）。
pub fn button_disabled<'a, Message: 'a + Clone>(
    label: impl Into<String>,
    colors: &ThemeColors,
) -> Element<'a, Message> {
    let colors = *colors;
    button(text(label.into()).size(14).color(colors.foreground_muted))
        .padding([6, 14])
        .style(move |_theme, _status| secondary_button_style(&colors, button::Status::Disabled))
        .into()
}

/// 纯文字按钮：主色文字 + 悬停浅底（用于「重试」等）。
pub fn text_button<'a, Message: Clone>(
    label: impl Into<String>,
    colors: &ThemeColors,
    on_press: Message,
) -> Button<'a, Message> {
    let colors = *colors;
    button(
        text(label.into())
            .size(14)
            .font(medium())
            .color(colors.primary),
    )
    .padding([8, 16])
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if hovered {
                Some(Background::Color(colors.primary_dim))
            } else {
                None
            },
            text_color: colors.primary,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(RADIUS_MD),
            },
            ..button::Style::default()
        }
    })
    .on_press(on_press)
}

// ---- labels / kbd ----

/// 普通文本标签。
pub fn label<'a, Message: 'a>(text_: &'a str, colors: &ThemeColors) -> Element<'a, Message> {
    text(text_.to_string())
        .size(14)
        .font(medium())
        .color(colors.foreground)
        .into()
}

/// 键盘按键样式（kbd）。
pub fn kbd<'a, Message: 'a>(key: &'a str, colors: &ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    container(
        text(key.to_string())
            .size(13)
            .color(colors.foreground_muted),
    )
    .padding([3, 8])
    .style(move |_| container::Style {
        background: Some(Background::Color(colors.surface_variant)),
        border: Border {
            color: colors.border,
            width: 1.0,
            radius: border::radius(6.0),
        },
        ..container::Style::default()
    })
    .into()
}

/// 状态胶囊：圆角标签。
pub fn badge<'a, Message: 'a>(label: &'a str, fg: Color, bg: Color) -> Element<'a, Message> {
    container(text(label.to_string()).size(12).color(fg))
        .padding([3, 10])
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(999.0),
            },
            ..container::Style::default()
        })
        .into()
}

// ---- progress ----

/// 细进度条（下载进度等）：轨道用底色，条用主色，圆角。
///
/// iced 自带的 progress_bar 样式在深色主题下跟卡片底色糊在一起，这里显式
/// 指定三段（轨道 / 条 / 圆角），粗细压到 6px——它是卡片里的**一行信息**，
/// 不该抢标题的视觉权重。
pub fn progress<'a, Message: 'a>(value: f32, colors: &ThemeColors) -> Element<'a, Message> {
    let colors = *colors;
    iced::widget::progress_bar(0.0..=1.0, value.clamp(0.0, 1.0))
        // ProgressBar 的尺寸走 length（长度）/ girth（粗细），不是 width/height。
        .length(Length::Fill)
        .girth(Length::Fixed(6.0))
        .style(move |_theme| iced::widget::progress_bar::Style {
            background: Background::Color(colors.surface_variant),
            bar: Background::Color(colors.primary),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(999.0),
            },
        })
        .into()
}

// ---- pick_list ----

/// 下拉选择框样式：底色 + 边框 + 主色把手（与商店页版本选择器同款观感）。
///
/// iced 默认 pick_list 用它的主题色，和本应用的 `ThemeColors`（自定义深/浅色）
/// 不同源，混在卡片里会显得"借来的控件"，所以统一到这里描一遍。
pub fn pick_list_style(
    colors: &ThemeColors,
    _status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    iced::widget::pick_list::Style {
        text_color: colors.foreground,
        background: Background::Color(colors.surface_variant),
        border: Border {
            color: colors.border,
            width: 1.0,
            radius: border::radius(RADIUS_MD),
        },
        handle_color: colors.primary,
        placeholder_color: colors.foreground_muted,
    }
}

// ---- switch ----

/// 开关（toggler），样式贴合主题。
pub fn switch<'a, Message: Clone + 'a>(
    checked: bool,
    colors: &ThemeColors,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    let colors = *colors;
    toggler(checked)
        .on_toggle(on_toggle)
        .style(move |_theme, _status: TogglerStatus| toggler::Style {
            background: Background::Color(if checked {
                colors.primary
            } else {
                colors.border_strong
            }),
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: Background::Color(colors.on_primary),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: Some(colors.foreground),
            border_radius: Some(border::radius(999.0)),
            padding_ratio: 0.15,
        })
        .into()
}

// ---- number input ----

/// 数字输入框：− 值 + 三段式步进控件。
pub fn number_input<'a, Message: Clone + 'a>(
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    colors: &ThemeColors,
    on_change: impl Fn(f64) -> Message + 'a,
) -> Element<'a, Message> {
    let colors = *colors;
    let can_dec = value > min;
    let can_inc = value < max;

    let dec_btn = button(text("−").size(16).color(if can_dec {
        colors.foreground
    } else {
        colors.disabled
    }))
    .padding([5, 12])
    .style(move |_theme, status| number_button_style(&colors, status));
    let dec_btn = if can_dec {
        dec_btn.on_press(on_change((value - step).max(min)))
    } else {
        dec_btn
    };

    let inc_btn = button(text("+").size(16).color(if can_inc {
        colors.foreground
    } else {
        colors.disabled
    }))
    .padding([5, 12])
    .style(move |_theme, status| number_button_style(&colors, status));
    let inc_btn = if can_inc {
        inc_btn.on_press(on_change((value + step).min(max)))
    } else {
        inc_btn
    };

    let center = container(
        text(format!("{}", value as i32))
            .size(14)
            .color(colors.primary)
            .font(semibold()),
    )
    .width(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill);

    container(
        row![dec_btn, center, inc_btn]
            .width(Length::Fill)
            .padding(2)
            .align_y(Alignment::Center),
    )
    .width(140)
    .height(36)
    .style(move |_| container::Style {
        background: Some(Background::Color(colors.surface_variant)),
        border: Border {
            color: colors.border,
            width: 1.0,
            radius: border::radius(RADIUS_MD),
        },
        ..container::Style::default()
    })
    .into()
}
