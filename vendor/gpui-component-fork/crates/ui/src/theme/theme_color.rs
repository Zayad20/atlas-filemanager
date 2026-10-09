use std::sync::Arc;

use crate::{ThemeMode, theme::DEFAULT_THEME_COLORS};

use gpui::Hsla;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Theme colors used throughout the UI components.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct ThemeColor {
    /// Used for accents such as hover background on MenuItem, ListItem, etc.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub accent: Hsla,
    /// Used for accent text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub accent_foreground: Hsla,
    /// Accordion background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub accordion: Hsla,
    /// Accordion hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub accordion_hover: Hsla,
    /// Default background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub background: Hsla,
    /// Default border color
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub border: Hsla,
    /// Button primary background color, fallback to `primary`.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub button_primary: Hsla,
    /// Button primary active background color, fallback to `primary_active`.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub button_primary_active: Hsla,
    /// Button primary text color, fallback to `primary_foreground`.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub button_primary_foreground: Hsla,
    /// Button primary hover background color, fallback to `primary_hover`.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub button_primary_hover: Hsla,
    /// Background color for GroupBox.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub group_box: Hsla,
    /// Text color for GroupBox.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub group_box_foreground: Hsla,
    /// Input caret color (Blinking cursor).
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub caret: Hsla,
    /// Chart 1 color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_1: Hsla,
    /// Chart 2 color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_2: Hsla,
    /// Chart 3 color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_3: Hsla,
    /// Chart 4 color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_4: Hsla,
    /// Chart 5 color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_5: Hsla,
    /// Bullish color for candlestick charts (upward price movement).
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_bullish: Hsla,
    /// Bearish color for candlestick charts (downward price movement).
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub chart_bearish: Hsla,
    /// Danger background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub danger: Hsla,
    /// Danger active background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub danger_active: Hsla,
    /// Danger text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub danger_foreground: Hsla,
    /// Danger hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub danger_hover: Hsla,
    /// Description List label background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub description_list_label: Hsla,
    /// Description List label foreground color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub description_list_label_foreground: Hsla,
    /// Drag border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub drag_border: Hsla,
    /// Drop target background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub drop_target: Hsla,
    /// Default text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub foreground: Hsla,
    /// Info background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub info: Hsla,
    /// Info active background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub info_active: Hsla,
    /// Info text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub info_foreground: Hsla,
    /// Info hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub info_hover: Hsla,
    /// Border color for inputs such as Input, Select, etc.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub input: Hsla,
    /// Link text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub link: Hsla,
    /// Active link text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub link_active: Hsla,
    /// Hover link text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub link_hover: Hsla,
    /// Background color for List and ListItem.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list: Hsla,
    /// Background color for active ListItem.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list_active: Hsla,
    /// Border color for active ListItem.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list_active_border: Hsla,
    /// Stripe background color for even ListItem.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list_even: Hsla,
    /// Background color for List header.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list_head: Hsla,
    /// Hover background color for ListItem.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub list_hover: Hsla,
    /// Muted backgrounds such as Skeleton and Switch.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub muted: Hsla,
    /// Muted text color, as used in disabled text.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub muted_foreground: Hsla,
    /// Background color for Popover.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub popover: Hsla,
    /// Text color for Popover.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub popover_foreground: Hsla,
    /// Primary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub primary: Hsla,
    /// Active primary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub primary_active: Hsla,
    /// Primary text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub primary_foreground: Hsla,
    /// Hover primary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub primary_hover: Hsla,
    /// Progress bar background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub progress_bar: Hsla,
    /// Used for focus ring.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub ring: Hsla,
    /// Scrollbar background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub scrollbar: Hsla,
    /// Scrollbar thumb background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub scrollbar_thumb: Hsla,
    /// Scrollbar thumb hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub scrollbar_thumb_hover: Hsla,
    /// Secondary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub secondary: Hsla,
    /// Active secondary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub secondary_active: Hsla,
    /// Secondary text color, used for secondary Button text color or secondary text.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub secondary_foreground: Hsla,
    /// Hover secondary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub secondary_hover: Hsla,
    /// Input selection background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub selection: Hsla,
    /// Sidebar background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar: Hsla,
    /// Sidebar accent background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_accent: Hsla,
    /// Sidebar accent text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_accent_foreground: Hsla,
    /// Sidebar border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_border: Hsla,
    /// Sidebar text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_foreground: Hsla,
    /// Sidebar primary background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_primary: Hsla,
    /// Sidebar primary text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub sidebar_primary_foreground: Hsla,
    /// Skeleton background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub skeleton: Hsla,
    /// Slider bar background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub slider_bar: Hsla,
    /// Slider thumb background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub slider_thumb: Hsla,
    /// Success background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub success: Hsla,
    /// Success text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub success_foreground: Hsla,
    /// Success hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub success_hover: Hsla,
    /// Success active background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub success_active: Hsla,
    /// Switch background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub switch: Hsla,
    /// Switch thumb background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub switch_thumb: Hsla,
    /// Tab background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab: Hsla,
    /// Tab active background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab_active: Hsla,
    /// Tab active text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab_active_foreground: Hsla,
    /// TabBar background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab_bar: Hsla,
    /// TabBar segmented background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab_bar_segmented: Hsla,
    /// Tab text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tab_foreground: Hsla,
    /// Table background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table: Hsla,
    /// Table active item background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_active: Hsla,
    /// Table active item border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_active_border: Hsla,
    /// Stripe background color for even TableRow.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_even: Hsla,
    /// Table head background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_head: Hsla,
    /// Table head text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_head_foreground: Hsla,
    /// Table footer background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_foot: Hsla,
    /// Table footer text color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_foot_foreground: Hsla,
    /// Table item hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_hover: Hsla,
    /// Table row border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub table_row_border: Hsla,
    /// TitleBar background color, use for Window title bar.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub title_bar: Hsla,
    /// TitleBar border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub title_bar_border: Hsla,
    /// Background color for Tiles.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub tiles: Hsla,
    /// Warning background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub warning: Hsla,
    /// Warning active background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub warning_active: Hsla,
    /// Warning hover background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub warning_hover: Hsla,
    /// Warning foreground color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub warning_foreground: Hsla,
    /// Overlay background color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub overlay: Hsla,
    /// Window border color.
    ///
    /// # Platform specific:
    ///
    /// This is only works on Linux, other platforms we can't change the window border color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub window_border: Hsla,

    /// The base red color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub red: Hsla,
    /// The base red light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub red_light: Hsla,
    /// The base green color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub green: Hsla,
    /// The base green light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub green_light: Hsla,
    /// The base blue color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub blue: Hsla,
    /// The base blue light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub blue_light: Hsla,
    /// The base yellow color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub yellow: Hsla,
    /// The base yellow light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub yellow_light: Hsla,
    /// The base magenta color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub magenta: Hsla,
    /// The base magenta light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub magenta_light: Hsla,
    /// The base cyan color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub cyan: Hsla,
    /// The base cyan light color.
    #[schemars(with = "String")]
    #[serde(with = "crate::theme::color_serde")]
    pub cyan_light: Hsla,
}

impl ThemeColor {
    /// Get the default light theme colors.
    pub fn light() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Light].0.clone()
    }

    /// Get the default dark theme colors.
    pub fn dark() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Dark].0.clone()
    }
}
