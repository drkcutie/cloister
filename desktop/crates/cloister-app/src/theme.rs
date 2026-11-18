//! Design tokens, defined once. Every colour, size, radius and duration on screen comes from
//! here (spec #1, "UI and design system"). Corners are cut, not round; enclosure is drawn with
//! tone and a 1px line: no shadows, blur or gradients, and no red anywhere.

use std::borrow::Cow;
use std::time::Duration;

use gpui_kit::component::Theme as KitTheme;
use gpui_kit::*;

/// The colours of one theme. Day runs from Workday Start to Shutdown Time; Night covers
/// Evening Lockdown and Rest Days.
#[derive(Clone, Copy)]
pub struct Palette {
    pub ground: Hsla,
    pub surface: Hsla,
    /// Sunk and raised areas (sidebar, "what is so" notices).
    pub sunk: Hsla,
    pub line: Hsla,
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub ink_3: Hsla,
    /// Depth 4 by day.
    pub accent: Hsla,
    /// Records and eyebrows.
    pub brass: Hsla,
    pub brass_tint: Hsla,
    /// Brass on a Depth 5 band.
    pub brass_on_deep: Hsla,
    /// Text on Depth 3 to 5.
    pub on_deep: Hsla,
}

impl Global for Palette {}

fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

impl Palette {
    pub fn day() -> Self {
        Palette {
            ground: hex(0xEEF0EE),
            surface: hex(0xF7F8F6),
            sunk: hex(0xE3E6E3),
            line: hex(0xCDD2CE),
            ink: hex(0x17211F),
            ink_2: hex(0x47534F),
            ink_3: hex(0x56625E),
            accent: depth(4),
            brass: hex(0x7A5A1E),
            brass_tint: hex(0xEFE7D3),
            brass_on_deep: hex(0xC9A968),
            on_deep: hex(0xF7F8F6),
        }
    }

    /// Not switched to yet: the app settles into Night at Shutdown Time once the rules
    /// engine knows about Shutdown Time and Evening Lockdown.
    #[allow(dead_code)]
    pub fn night() -> Self {
        Palette {
            ground: hex(0x0E1716),
            surface: hex(0x15201F),
            sunk: hex(0x1C2A29),
            line: hex(0x2B3B39),
            ink: hex(0xE4EAE7),
            ink_2: hex(0xA9B7B3),
            ink_3: hex(0xA9B7B3),
            accent: hex(0x8FB8B2),
            brass: hex(0xC9A968),
            brass_tint: hex(0x2B3B39),
            brass_on_deep: hex(0xC9A968),
            on_deep: hex(0xE4EAE7),
        }
    }
}

/// The depth ramp: one hue, five depths. Also the Depth Rating scale.
/// 1 selected rows and tints, 2 shallow fill, 3 focus ring, 4 commit actions and Deep Hours,
/// 5 hero bands and the committed state.
pub fn depth(level: u8) -> Hsla {
    hex(match level {
        1 => 0xC9D9D6,
        2 => 0x94B3AF,
        3 => 0x4A7A76,
        4 => 0x2F5F5E,
        _ => 0x123837,
    })
}

/// Ink on a depth swatch: dark on 1 and 2, light on 3 to 5 (WCAG AA either way).
pub fn ink_on_depth(level: u8, palette: &Palette) -> Hsla {
    if level <= 2 {
        hex(0x17211F)
    } else {
        palette.on_deep
    }
}

/// Spacing on a 4px base (the full scale; not every step is used yet).
#[allow(dead_code)]
pub mod space {
    use gpui_kit::{px, Pixels};
    pub const S1: Pixels = px(4.);
    pub const S2: Pixels = px(8.);
    pub const S3: Pixels = px(12.);
    pub const S4: Pixels = px(16.);
    pub const S6: Pixels = px(24.);
    pub const S8: Pixels = px(32.);
    pub const S12: Pixels = px(48.);
    pub const S16: Pixels = px(64.);
    /// Minimum touch / click target.
    pub const TARGET: Pixels = px(48.);
}

/// Corner radii. No pills or circles.
pub mod radius {
    use gpui_kit::{px, Pixels};
    /// Bands and the Depth Rating.
    pub const NONE: Pixels = px(0.);
    /// Buttons, fields, tags.
    pub const CONTROL: Pixels = px(2.);
    /// Cards and panels.
    pub const CARD: Pixels = px(4.);
}

/// Motion. Nothing bounces or celebrates; timers tick with no animation.
pub mod motion {
    use super::Duration;
    /// State and screen changes, ease-out.
    pub const FADE: Duration = Duration::from_millis(240);
    /// Hold to commit, linear. Mirrors `cloister_app_model::Hold::DURATION`, which drives it.
    #[allow(dead_code)]
    pub const SINK: Duration = Duration::from_millis(2000);
    /// Day to Night, ease-out.
    #[allow(dead_code)]
    pub const SETTLE: Duration = Duration::from_millis(600);
}

pub const SERIF: &str = "EB Garamond";
pub const SANS: &str = "Source Sans 3";
pub const MONO: &str = "Source Code Pro";

/// The type scale. Each style sets family, weight, size and line height on an element.
#[derive(Clone, Copy)]
pub enum Type {
    /// EB Garamond 500 52/56.
    Display,
    /// EB Garamond 500 34/38.
    Title,
    /// EB Garamond 400 30/38. The Intent is the only serif input.
    Intent,
    /// Source Sans 3 600 20/28.
    Heading,
    /// Source Sans 3 400 16/24.
    Body,
    /// Source Sans 3 500 13/20.
    Label,
    /// Source Sans 3 600 12/16, capitals (the only capitals).
    Eyebrow,
    /// Source Code Pro 500 64/64.
    Timer,
    /// Source Code Pro 500, any size: figures.
    Figure(f32),
}

impl Type {
    pub fn apply<E: Styled>(self, el: E) -> E {
        let (family, weight, size, line) = match self {
            Type::Display => (SERIF, FontWeight::MEDIUM, 52., 56.),
            Type::Title => (SERIF, FontWeight::MEDIUM, 34., 38.),
            Type::Intent => (SERIF, FontWeight::NORMAL, 30., 38.),
            Type::Heading => (SANS, FontWeight::SEMIBOLD, 20., 28.),
            Type::Body => (SANS, FontWeight::NORMAL, 16., 24.),
            Type::Label => (SANS, FontWeight::MEDIUM, 13., 20.),
            Type::Eyebrow => (SANS, FontWeight::SEMIBOLD, 12., 16.),
            Type::Timer => (MONO, FontWeight::MEDIUM, 64., 64.),
            Type::Figure(size) => (MONO, FontWeight::MEDIUM, size, size * 1.25),
        };
        el.font_family(family)
            .font_weight(weight)
            .text_size(px(size))
            .line_height(px(line))
    }
}

/// Load the bundled fonts (OFL; licences next to them in `assets/fonts`).
pub fn load_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/EBGaramond-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/EBGaramond-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/SourceSans3-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/SourceSans3-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/SourceCodePro-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/SourceCodePro-Medium.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("cloister-app: loading fonts failed: {e}");
    }
}

/// Install the Day palette and align gpui-kit's own theme (used by the text input) with it.
pub fn install(cx: &mut App) {
    let palette = Palette::day();
    cx.set_global(palette);
    let kit = KitTheme::global_mut(cx);
    kit.font_family = SANS.into();
    kit.font_size = px(16.);
    kit.mono_font_family = MONO.into();
    kit.radius = radius::CONTROL;
    kit.radius_lg = radius::CARD;
    kit.shadow = false;
    kit.background = palette.ground;
    kit.foreground = palette.ink;
    kit.muted_foreground = palette.ink_3;
    kit.border = palette.line;
    kit.input = palette.ink;
    kit.ring = depth(3);
    kit.caret = palette.ink;
    kit.selection = depth(1);
}

pub trait PaletteExt {
    fn palette(&self) -> Palette;
}

impl PaletteExt for App {
    fn palette(&self) -> Palette {
        *self.global::<Palette>()
    }
}
