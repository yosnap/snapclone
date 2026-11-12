//! The style the editor stores in `settings.json`, read with the same rules as the editor's
//! `parseStoredStyle`: every field is validated on its own and an invalid one falls back to
//! the default, so a hand-edited or older file still composes like the editor shows it.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

/// Upper bounds of the numeric controls, in points.
const PADDING_MAX: f64 = 300.0;
const INSET_MAX: f64 = 200.0;
const RADIUS_MAX: f64 = 50.0;
const SHADOW_MAX: f64 = 100.0;
/// Counted in UTF-16 code units, like JavaScript's `slice`.
const WATERMARK_MAX_LENGTH: usize = 80;
const DEFAULT_WATERMARK: &str = "Captura hecha con Snapclone";
/// The gradient a new install starts with ("Cool").
const DEFAULT_GRADIENT: usize = 5;
const SIZE_MIN: u32 = 16;
const SIZE_MAX: u32 = 8000;
const RATIO_MAX: f64 = 100.0;

/// A CSS colour as the canvas holds it: 8-bit channels, alpha included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    pub const fn hex(value: u32) -> Self {
        Self { red: (value >> 16) as u8, green: (value >> 8) as u8, blue: value as u8, alpha: 255 }
    }

    /// `rgba(r, g, b, a)`; WebKit keeps legacy colours at 8 bits, so alpha is quantised too.
    #[cfg(test)]
    pub fn rgba(red: u8, green: u8, blue: u8, alpha: f64) -> Self {
        Self { red, green, blue, alpha: js_round(alpha.clamp(0.0, 1.0) * 255.0) as u8 }
    }

    /// `#rrggbb`, the only form solid backgrounds are stored in.
    pub fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok().map(Self::hex)
    }

    /// Channels as CoreGraphics takes them, in 0…1.
    pub fn components(self) -> [f64; 4] {
        [self.red, self.green, self.blue, self.alpha].map(|channel| f64::from(channel) / 255.0)
    }
}

/// A soft radial highlight over a gradient; position and radius are fractions of the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub color: Color,
}

/// A CSS linear gradient (0° points up) with evenly spaced stops, plus optional glows.
#[derive(Debug, PartialEq)]
pub struct Gradient {
    pub angle: f64,
    pub colors: &'static [Color],
    pub glows: &'static [Glow],
}

impl Gradient {
    /// Stop offsets in 0…1, spread evenly like the editor's `gradient()` helper.
    pub fn stops(&self) -> impl Iterator<Item = (f64, Color)> + '_ {
        let last = self.colors.len().saturating_sub(1).max(1) as f64;
        self.colors.iter().enumerate().map(move |(index, color)| (index as f64 / last, *color))
    }
}

const fn gradient(angle: f64, colors: &'static [Color], glows: &'static [Glow]) -> Gradient {
    Gradient { angle, colors, glows }
}

/// Every gradient, by index, in the editor's order: stored styles refer to them by index.
pub static GRADIENT_PRESETS: [Gradient; 14] = [
    gradient(45.0, &[Color::hex(0x6a2c91), Color::hex(0xd6457a), Color::hex(0xf08a4b)], &[]),
    gradient(45.0, &[Color::hex(0x0f9b8e), Color::hex(0x3ad6a4)], &[]),
    gradient(45.0, &[Color::hex(0xe97bb4), Color::hex(0x7a7ff0)], &[]),
    gradient(45.0, &[Color::hex(0xf6c89a), Color::hex(0xe8a3e6)], &[]),
    gradient(45.0, &[Color::hex(0xf4a66a), Color::hex(0xf07c84)], &[]),
    gradient(150.0, &[Color::hex(0x1f8ef1), Color::hex(0x7fd6cf)], &[]),
    gradient(100.0, &[Color::hex(0xe8870c), Color::hex(0xffc36b)], &[]),
    gradient(100.0, &[Color::hex(0xee5a7a), Color::hex(0xa43f9e)], &[]),
    gradient(100.0, &[Color::hex(0x9b5de5), Color::hex(0x4f6fe0)], &[]),
    gradient(
        160.0,
        &[Color::hex(0x16b8a0), Color::hex(0x22c7b4), Color::hex(0x5fd8cc)],
        &[Glow { x: 0.2, y: 0.15, radius: 0.6, color: Color { red: 255, green: 255, blue: 255, alpha: 71 } }],
    ),
    gradient(
        150.0,
        &[Color::hex(0xf7a1c4), Color::hex(0xf7797d), Color::hex(0xfbb48c)],
        &[Glow { x: 0.1, y: 0.95, radius: 0.55, color: Color { red: 150, green: 190, blue: 255, alpha: 191 } }],
    ),
    gradient(
        150.0,
        &[Color::hex(0x4b1d8f), Color::hex(0x9b2bb1), Color::hex(0xd9479a)],
        &[Glow { x: 0.75, y: 0.3, radius: 0.5, color: Color { red: 255, green: 90, blue: 170, alpha: 115 } }],
    ),
    gradient(
        135.0,
        &[Color::hex(0xd9c2f5), Color::hex(0xf1b7e0), Color::hex(0xb9c8f7)],
        &[Glow { x: 0.3, y: 0.7, radius: 0.5, color: Color { red: 255, green: 255, blue: 255, alpha: 128 } }],
    ),
    gradient(
        170.0,
        &[Color::hex(0x5a8dee), Color::hex(0x77c6f4), Color::hex(0xb6e3f7)],
        &[Glow { x: 0.85, y: 0.85, radius: 0.55, color: Color { red: 244, green: 170, blue: 220, alpha: 153 } }],
    ),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// An index into [`GRADIENT_PRESETS`], always in range.
    Gradient(usize),
    Solid(Color),
    /// The main display's desktop picture, blurred.
    Wallpaper,
    /// A transparent backdrop.
    None,
}

/// "auto", a ratio such as 16:9, or an exact output size in pixels such as 1600x900.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ratio {
    Auto,
    Proportion { width: f64, height: f64 },
    Size { width: u32, height: u32 },
}

impl Ratio {
    /// Parses "auto", "4:3" or "1920x1080"; anything else is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        static RATIO: OnceLock<Regex> = OnceLock::new();
        static SIZE: OnceLock<Regex> = OnceLock::new();
        let text = value.trim().to_lowercase();
        if text == "auto" {
            return Some(Self::Auto);
        }
        let ratio = RATIO.get_or_init(|| Regex::new(r"^([0-9]+(?:\.[0-9]+)?)\s*:\s*([0-9]+(?:\.[0-9]+)?)$").expect("valid regex"));
        if let Some(captures) = ratio.captures(&text) {
            let width: f64 = captures[1].parse().ok()?;
            let height: f64 = captures[2].parse().ok()?;
            let valid = width > 0.0 && height > 0.0 && width <= RATIO_MAX && height <= RATIO_MAX;
            return valid.then_some(Self::Proportion { width, height });
        }
        let size = SIZE.get_or_init(|| Regex::new(r"^([0-9]+)\s*[x×]\s*([0-9]+)$").expect("valid regex"));
        let captures = size.captures(&text)?;
        let width: u32 = captures[1].parse().ok()?;
        let height: u32 = captures[2].parse().ok()?;
        let valid = [width, height].iter().all(|side| (SIZE_MIN..=SIZE_MAX).contains(side));
        valid.then_some(Self::Size { width, height })
    }

    /// Width over height, or `None` for "auto".
    pub fn value(self) -> Option<f64> {
        match self {
            Self::Auto => None,
            Self::Proportion { width, height } => Some(width / height),
            Self::Size { width, height } => Some(f64::from(width) / f64::from(height)),
        }
    }

    /// The exact output size in pixels when the ratio is a size.
    pub fn output_size(self) -> Option<(u32, u32)> {
        match self {
            Self::Size { width, height } => Some((width, height)),
            _ => None,
        }
    }
}

/// All lengths are in points; the compositor multiplies them by the capture scale.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredStyle {
    pub background: Background,
    pub padding: f64,
    pub inset: f64,
    pub radius: f64,
    pub shadow: f64,
    pub ratio: Ratio,
    pub watermark: String,
    pub show_watermark: bool,
}

impl Default for StoredStyle {
    fn default() -> Self {
        Self {
            background: Background::Gradient(DEFAULT_GRADIENT),
            padding: 100.0,
            inset: 0.0,
            radius: 20.0,
            shadow: 25.0,
            ratio: Ratio::Auto,
            watermark: DEFAULT_WATERMARK.into(),
            show_watermark: false,
        }
    }
}

impl StoredStyle {
    /// Validates untrusted JSON; `None` when it is not an object at all, like the editor.
    pub fn parse(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let fallback = Self::default();
        let number = |key: &str, max: f64, default: f64| clamp_number(object.get(key), max).unwrap_or(default);
        let watermark = object.get("watermark").and_then(Value::as_str).map(truncate_utf16);
        let has_text = watermark.as_deref().is_some_and(|text| !text.trim().is_empty());
        Some(Self {
            background: object.get("background").and_then(parse_background).unwrap_or(fallback.background),
            padding: number("padding", PADDING_MAX, fallback.padding),
            inset: number("inset", INSET_MAX, fallback.inset),
            radius: number("radius", RADIUS_MAX, fallback.radius),
            shadow: number("shadow", SHADOW_MAX, fallback.shadow),
            ratio: object.get("ratio").and_then(Value::as_str).and_then(Ratio::parse).unwrap_or(fallback.ratio),
            // Styles saved before the checkbox existed showed any non-empty watermark.
            show_watermark: object.get("showWatermark").and_then(Value::as_bool).unwrap_or(has_text),
            watermark: if has_text { watermark.unwrap_or_default() } else { fallback.watermark },
        })
    }

    /// The text drawn below the card, if any.
    pub fn watermark_text(&self) -> Option<&str> {
        let text = self.watermark.trim();
        (self.show_watermark && !text.is_empty()).then_some(text)
    }
}

/// An image background lives only in the editor's session and is never stored; like any
/// other unknown value it is not accepted here, so the default gradient applies.
fn parse_background(value: &Value) -> Option<Background> {
    let object = value.as_object()?;
    match object.get("kind")?.as_str()? {
        "solid" => object.get("color")?.as_str().and_then(Color::parse_hex).map(Background::Solid),
        "wallpaper" => Some(Background::Wallpaper),
        "none" => Some(Background::None),
        "gradient" => {
            let index = object.get("index")?;
            // `Number.isInteger` accepts 3.0 as well as 3.
            let index = index.as_u64().or_else(|| index.as_f64().filter(|value| value.fract() == 0.0 && *value >= 0.0).map(|value| value as u64))?;
            let index = usize::try_from(index).ok()?;
            (index < GRADIENT_PRESETS.len()).then_some(Background::Gradient(index))
        }
        _ => None,
    }
}

fn clamp_number(value: Option<&Value>, max: f64) -> Option<f64> {
    let value = value?.as_f64().filter(|value| value.is_finite())?;
    Some(js_round(value).clamp(0.0, max))
}

/// JavaScript's `slice(0, 80)` counts UTF-16 code units; a character that would straddle
/// the limit is left out rather than split.
fn truncate_utf16(text: &str) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|character| {
            units += character.len_utf16();
            units <= WATERMARK_MAX_LENGTH
        })
        .collect()
}

/// `Math.round`: halves round towards positive infinity.
pub fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_stored_style() {
        let stored = json!({"background":{"kind":"wallpaper"},"inset":104,"padding":44,"radius":13,"ratio":"auto","shadow":22,"showWatermark":true,"watermark":"Captura hecha con Snapclone"});
        let style = StoredStyle::parse(&stored).unwrap();
        assert_eq!(style.background, Background::Wallpaper);
        assert_eq!((style.padding, style.inset, style.radius, style.shadow), (44.0, 104.0, 13.0, 22.0));
        assert_eq!(style.ratio, Ratio::Auto);
        assert_eq!(style.watermark_text(), Some("Captura hecha con Snapclone"));
    }

    #[test]
    fn invalid_fields_fall_back_one_by_one() {
        let style = StoredStyle::parse(&json!({"padding": "x", "inset": 999, "radius": -4, "shadow": 12.5, "ratio": "nope", "background": {"kind": "image"}})).unwrap();
        let fallback = StoredStyle::default();
        assert_eq!(style.padding, fallback.padding);
        assert_eq!(style.inset, INSET_MAX);
        assert_eq!(style.radius, 0.0);
        assert_eq!(style.shadow, 13.0, "Math.round rounds halves up");
        assert_eq!(style.ratio, Ratio::Auto);
        assert_eq!(style.background, Background::Gradient(DEFAULT_GRADIENT));
        assert!(StoredStyle::parse(&json!([1, 2])).is_none());
        assert!(StoredStyle::parse(&json!(null)).is_none());
    }

    #[test]
    fn backgrounds_are_validated() {
        let parse = |value| parse_background(&value);
        assert_eq!(parse(json!({"kind": "solid", "color": "#FF8000"})), Some(Background::Solid(Color::hex(0xff8000))));
        assert_eq!(parse(json!({"kind": "solid", "color": "red"})), None);
        assert_eq!(parse(json!({"kind": "gradient", "index": 13})), Some(Background::Gradient(13)));
        assert_eq!(parse(json!({"kind": "gradient", "index": 4.0})), Some(Background::Gradient(4)));
        assert_eq!(parse(json!({"kind": "gradient", "index": 14})), None);
        assert_eq!(parse(json!({"kind": "gradient", "index": 1.5})), None);
        assert_eq!(parse(json!({"kind": "gradient", "index": -1})), None);
        assert_eq!(parse(json!({"kind": "none"})), Some(Background::None));
    }

    #[test]
    fn watermark_rules_match_the_editor() {
        let legacy = StoredStyle::parse(&json!({"watermark": "  hola  "})).unwrap();
        assert!(legacy.show_watermark, "a non-empty watermark without the checkbox is shown");
        assert_eq!(legacy.watermark_text(), Some("hola"));
        let blank = StoredStyle::parse(&json!({"watermark": "   ", "showWatermark": true})).unwrap();
        assert_eq!(blank.watermark, DEFAULT_WATERMARK);
        let hidden = StoredStyle::parse(&json!({"watermark": "x", "showWatermark": false})).unwrap();
        assert_eq!(hidden.watermark_text(), None);
        let long = StoredStyle::parse(&json!({"watermark": "é".repeat(100)})).unwrap();
        assert_eq!(long.watermark.chars().count(), WATERMARK_MAX_LENGTH);
        let emoji = truncate_utf16(&format!("{}😀", "a".repeat(79)));
        assert_eq!(emoji, "a".repeat(79), "a surrogate pair is never split");
    }

    #[test]
    fn ratios_parse_like_the_editor() {
        assert_eq!(Ratio::parse(" AUTO "), Some(Ratio::Auto));
        assert_eq!(Ratio::parse("16:9").and_then(Ratio::value), Some(16.0 / 9.0));
        assert_eq!(Ratio::parse("1.5 : 1"), Some(Ratio::Proportion { width: 1.5, height: 1.0 }));
        assert_eq!(Ratio::parse("0:1"), None);
        assert_eq!(Ratio::parse("101:1"), None);
        assert_eq!(Ratio::parse("1600x900"), Some(Ratio::Size { width: 1600, height: 900 }));
        assert_eq!(Ratio::parse("1080 × 1920").and_then(Ratio::output_size), Some((1080, 1920)));
        assert_eq!(Ratio::parse("15x900"), None);
        assert_eq!(Ratio::parse("8001x900"), None);
        assert_eq!(Ratio::parse("١٦:٩"), None, "only ASCII digits, like JavaScript's \\d");
        assert_eq!(Ratio::parse("4:3").and_then(Ratio::output_size), None);
        assert_eq!(Ratio::Auto.value(), None);
    }

    #[test]
    fn colours_match_the_canvas() {
        assert_eq!(Color::rgba(255, 255, 255, 0.28), GRADIENT_PRESETS[9].glows[0].color);
        assert_eq!(Color::rgba(150, 190, 255, 0.75), GRADIENT_PRESETS[10].glows[0].color);
        assert_eq!(Color::rgba(255, 90, 170, 0.45), GRADIENT_PRESETS[11].glows[0].color);
        assert_eq!(Color::rgba(255, 255, 255, 0.5), GRADIENT_PRESETS[12].glows[0].color);
        assert_eq!(Color::rgba(244, 170, 220, 0.6), GRADIENT_PRESETS[13].glows[0].color);
        let stops: Vec<f64> = GRADIENT_PRESETS[0].stops().map(|(offset, _)| offset).collect();
        assert_eq!(stops, [0.0, 0.5, 1.0]);
        assert_eq!(GRADIENT_PRESETS[1].stops().last().map(|(offset, _)| offset), Some(1.0));
    }
}
