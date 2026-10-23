use crate::components::settings::{settings_group, settings_item, settings_page};
use crate::components::widgets::{
    button_danger, button_disabled, button_primary, button_secondary, number_input,
    pick_list_style,
};
use crate::state::{Message, SettingsState};
use crate::theme::ThemeColors;
use iced::widget::{pick_list, row};
use iced::Element;

/// Dark mode options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkModeOption {
    Light,
    Dark,
    FollowSystem,
}

impl DarkModeOption {
    pub fn label(&self) -> &str {
        match self {
            DarkModeOption::Light => "浅色",
            DarkModeOption::Dark => "深色",
            DarkModeOption::FollowSystem => "跟随系统",
        }
    }

    pub fn from_value(v: u8) -> Self {
        match v {
            0 => DarkModeOption::Light,
            1 => DarkModeOption::Dark,
            _ => DarkModeOption::FollowSystem,
        }
    }

    pub fn to_value(&self) -> u8 {
        match self {
            DarkModeOption::Light => 0,
            DarkModeOption::Dark => 1,
            DarkModeOption::FollowSystem => 2,
        }
    }
}

impl std::fmt::Display for DarkModeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

pub fn view<'a>(settings: &'a SettingsState, colors: &'a ThemeColors) -> Element<'a, Message> {
    let appearance = &settings.appearance;

    // Get available color scheme names for pick lists
    let scheme_names: Vec<String> = appearance
        .available_color_schemes
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect();

    // Get current light/dark scheme names
    let (light_scheme, dark_scheme) = match &appearance.color_scheme {
        xime_config::ColorSchemeConfig::Simple(name) => (name.clone(), name.clone()),
        xime_config::ColorSchemeConfig::Named { light, dark } => (light.clone(), dark.clone()),
    };

    let dark_mode_option = DarkModeOption::from_value(appearance.dark_mode.value());

    settings_page(
        "外观",
        colors,
        vec![
            settings_group(
                "主题",
                None::<String>,
                colors,
                vec![
                    settings_item(
                        "外观模式",
                        Some("浅色 / 深色 / 跟随系统"),
                        colors,
                        pick_list(
                            vec![
                                DarkModeOption::Light,
                                DarkModeOption::Dark,
                                DarkModeOption::FollowSystem,
                            ],
                            Some(dark_mode_option),
                            |mode| Message::DarkModeChanged(mode.to_value()),
                        )
                        .style(move |_theme, status| pick_list_style(colors, status))
                        .into(),
                    ),
                    settings_item(
                        "浅色配色",
                        Some("浅色模式下使用的配色方案"),
                        colors,
                        pick_list(
                            scheme_names.clone(),
                            Some(light_scheme),
                            Message::ColorSchemeLightChanged,
                        )
                        .style(move |_theme, status| pick_list_style(colors, status))
                        .into(),
                    ),
                    settings_item(
                        "深色配色",
                        Some("深色模式下使用的配色方案"),
                        colors,
                        pick_list(
                            scheme_names,
                            Some(dark_scheme),
                            Message::ColorSchemeDarkChanged,
                        )
                        .style(move |_theme, status| pick_list_style(colors, status))
                        .into(),
                    ),
                ],
            ),
            settings_group(
                "显示",
                None::<String>,
                colors,
                vec![
                    settings_item(
                        "字号",
                        Some("候选词显示字号"),
                        colors,
                        number_input(
                            appearance.font_size,
                            8.0,
                            48.0,
                            1.0,
                            colors,
                            Message::FontSizeChanged,
                        ),
                    ),
                    settings_item(
                        "候选词数量",
                        Some("候选词列表中显示的数量（1–10，第 10 个用 0 选）"),
                        colors,
                        number_input(
                            appearance.candidate_count as f64,
                            1.0,
                            10.0,
                            1.0,
                            colors,
                            |v| Message::CandidateCountChanged(v as i32),
                        ),
                    ),
                    settings_item(
                        "圆角大小",
                        Some("候选窗口圆角半径"),
                        colors,
                        number_input(
                            appearance.corner_radius,
                            0.0,
                            24.0,
                            1.0,
                            colors,
                            Message::CornerRadiusChanged,
                        ),
                    ),
                ],
            ),
            settings_group(
                "操作",
                None::<String>,
                colors,
                vec![
                    row![button_primary(
                        "保存外观设置",
                        colors,
                        Message::SaveAppearance
                    )]
                    .into(),
                    // 配置文件本身的操作：撤销上一次修改 / 恢复默认（删掉用户文件）/
                    // 恢复为用户最初提供的那份（`.orig`，仅在程序改写前存在过时可用）。
                    {
                        let file_state = SettingsState::config_file_state();
                        let undo = if file_state.backup {
                            button_secondary("撤销上一次修改", colors, Message::UndoConfigChange)
                                .into()
                        } else {
                            button_disabled("撤销上一次修改", colors)
                        };
                        let restore_original = if file_state.original {
                            button_secondary(
                                "恢复为用户提供的配置",
                                colors,
                                Message::RestoreConfigOriginal,
                            )
                            .into()
                        } else {
                            button_disabled("恢复为用户提供的配置", colors)
                        };
                        let restore_defaults = if file_state.user_file {
                            button_danger("恢复默认配置", colors, Message::RestoreConfigDefaults)
                                .into()
                        } else {
                            button_disabled("恢复默认配置", colors)
                        };
                        row![undo, restore_original, restore_defaults].spacing(8).into()
                    },
                ],
            ),
        ],
    )
}
