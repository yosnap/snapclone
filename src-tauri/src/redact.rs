//! Automatic redaction: Vision reads the capture's text (see `ocr`) and regexes choose what to hide.
//! Patterns start from the ones Xnapper ships, tightened where they misfire on ordinary
//! text (times read as IPv6, any digit run read as a card number).

use std::{ops::Range, sync::LazyLock};

use regex::Regex;
use serde::Serialize;
use tauri::AppHandle;

use crate::ocr::{self, NormalizedRect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Email,
    CreditCard,
    IpAddress,
    ApiKey,
}

/// Normalized (0..1) box with the origin at the capture's top-left corner.
#[derive(Clone, Debug, Serialize)]
pub struct SensitiveRegion {
    category: Category,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl SensitiveRegion {
    pub fn new(category: Category, rect: NormalizedRect) -> Self {
        Self { category, x: rect.x, y: rect.y, width: rect.width, height: rect.height }
    }
}

/// Regexes whose capture group `group` is the sensitive text; group 0 is the whole match.
/// Group 1 is used where the Rust regex engine's lack of look-behind needs a consumed prefix.
static PATTERNS: LazyLock<Vec<(Category, Regex, usize)>> = LazyLock::new(|| {
    let ipv4_octet = r"(?:25[0-5]|2[0-4]\d|1?\d?\d)";
    [
        (Category::Email, r"(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}".to_string(), 0),
        (Category::IpAddress, format!(r"\b(?:{ipv4_octet}\.){{3}}{ipv4_octet}\b"), 0),
        // Full IPv6, or compressed with `::` and something after it, so clock times (12:30),
        // MAC addresses and Rust paths (`std::fs`, `add::`) never match.
        (Category::IpAddress, r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b".to_string(), 0),
        (
            Category::IpAddress,
            r"(?i)(?:^|[^0-9a-z:])((?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}|::(?:[0-9a-f]{1,4}:){0,5}[0-9a-f]{1,4})\b"
                .to_string(),
            1,
        ),
        // No `/`: it turned file paths into "keys". `+` and `=` padding cover base64 secrets.
        (Category::ApiKey, r"[A-Za-z0-9_+\-]{32,}={0,2}".to_string(), 0),
    ]
    .into_iter()
    .map(|(category, pattern, group)| (category, Regex::new(&pattern).expect("static pattern is valid"), group))
    .collect()
});

/// Runs of digit groups separated by single spaces or dashes, as card numbers are printed.
static DIGIT_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:[ -]\d+)*").expect("static pattern is valid"));
static DIGIT_GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").expect("static pattern is valid"));

/// Byte ranges of sensitive substrings in one recognized line.
pub fn find_sensitive(text: &str) -> Vec<(Category, Range<usize>)> {
    let mut found: Vec<(Category, Range<usize>)> = PATTERNS
        .iter()
        .flat_map(|(category, regex, group)| {
            regex
                .captures_iter(text)
                .filter_map(move |captures| captures.get(*group))
                .map(move |matched| (*category, matched))
        })
        .filter(|(category, matched)| *category != Category::ApiKey || is_key_like(matched.as_str()))
        .map(|(category, matched)| (category, matched.range()))
        .collect();
    found.extend(card_numbers(text).into_iter().map(|range| (Category::CreditCard, range)));
    found
}

/// Longest Luhn-valid span of 13–19 digits starting at each group, so trailing digits
/// on the same line (a CVV, an expiry) do not hide the card number before them.
fn card_numbers(text: &str) -> Vec<Range<usize>> {
    let mut cards = Vec::new();
    for run in DIGIT_RUN.find_iter(text) {
        let groups: Vec<Range<usize>> = DIGIT_GROUP
            .find_iter(run.as_str())
            .map(|group| run.start() + group.start()..run.start() + group.end())
            .collect();
        let mut first = 0;
        while first < groups.len() {
            let mut digits = 0;
            let mut longest = None;
            for (last, group) in groups.iter().enumerate().skip(first) {
                digits += group.len();
                if digits > 19 {
                    break;
                }
                let span = groups[first].start..group.end;
                if digits >= 13 && passes_luhn(&text[span.clone()]) {
                    longest = Some((last, span));
                }
            }
            match longest {
                Some((last, span)) => {
                    cards.push(span);
                    first = last + 1;
                }
                None => first += 1,
            }
        }
    }
    cards
}

fn passes_luhn(candidate: &str) -> bool {
    let sum: u32 = candidate
        .chars()
        .filter_map(|c| c.to_digit(10))
        .rev()
        .enumerate()
        .map(|(index, digit)| match index % 2 {
            0 => digit,
            _ if digit * 2 > 9 => digit * 2 - 9,
            _ => digit * 2,
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Keys mix letters and digits; long plain words are not keys. UUIDs still count,
/// because many services issue them as secrets and a false positive is cheap to delete.
fn is_key_like(candidate: &str) -> bool {
    candidate.chars().any(|c| c.is_ascii_digit()) && candidate.chars().any(|c| c.is_ascii_alphabetic())
}

/// Returns every sensitive region in the capture; the editor filters by enabled category.
/// Vision runs in `ocr`, whose cached pass also serves "Texto detectado".
#[tauri::command]
pub async fn detect_sensitive(app: AppHandle, id: String) -> Result<Vec<SensitiveRegion>, String> {
    Ok(ocr::analyze(&app, &id).await?.sensitive.clone())
}

#[cfg(test)]
mod tests {
    use super::{Category, find_sensitive};

    fn categories(text: &str) -> Vec<(Category, &str)> {
        find_sensitive(text).into_iter().map(|(category, range)| (category, &text[range])).collect()
    }

    #[test]
    fn finds_emails() {
        assert_eq!(categories("Escríbeme a ana.lopez@ejemplo.es hoy"), vec![(Category::Email, "ana.lopez@ejemplo.es")]);
    }

    #[test]
    fn finds_luhn_valid_cards_only() {
        assert_eq!(categories("Tarjeta 4111 1111 1111 1111"), vec![(Category::CreditCard, "4111 1111 1111 1111")]);
        assert!(categories("Pedido 4111 1111 1111 1112").is_empty());
        assert!(categories("Tel 600 123 456").is_empty());
    }

    #[test]
    fn finds_ip_addresses_but_not_times() {
        assert_eq!(categories("host 192.168.1.20 up"), vec![(Category::IpAddress, "192.168.1.20")]);
        assert_eq!(categories("via fe80::1ff:fe23:4567:890a"), vec![(Category::IpAddress, "fe80::1ff:fe23:4567:890a")]);
        assert!(categories("Reunión a las 12:30 o 09:15:00").is_empty());
        assert!(categories("version 1.2.3.400").is_empty());
    }

    #[test]
    fn finds_cards_followed_by_more_digits() {
        assert_eq!(categories("Tarjeta 4111 1111 1111 1111 123"), vec![(Category::CreditCard, "4111 1111 1111 1111")]);
        assert_eq!(categories("Ref 12 4111 1111 1111 1111"), vec![(Category::CreditCard, "4111 1111 1111 1111")]);
    }

    #[test]
    fn finds_compressed_ipv6_but_not_code_paths() {
        assert_eq!(categories("loopback ::1 ok"), vec![(Category::IpAddress, "::1")]);
        assert!(categories("use std::fs; let x = add::new();").is_empty());
        assert!(categories("mac aa:bb:cc:dd:ee:ff").is_empty());
    }

    #[test]
    fn paths_are_not_keys_but_base64_secrets_are() {
        assert!(categories("/Users/paulo/Proyectos/app2/src/main.rs").is_empty());
        let secret = "dGhpc2lzYWZha2VzZWNyZXQxMjM0NTY3ODkw+abc==";
        assert_eq!(categories(secret), vec![(Category::ApiKey, secret)]);
    }

    #[test]
    fn finds_keys_with_letters_and_digits() {
        let key = "q7Rt2vX9bN4mK8pL3sD6fG1hJ5wZ0cYeA";
        assert_eq!(categories(&format!("KEY={key}")), vec![(Category::ApiKey, key)]);
        assert!(categories("Supercalifragilisticexpialidociousword").is_empty());
    }

    /// Runs real Vision OCR: `REDACT_SAMPLE=/path/to.png cargo test -- --ignored --nocapture`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a sample image in REDACT_SAMPLE"]
    fn vision_finds_regions_in_sample_image() {
        let path = std::env::var("REDACT_SAMPLE").expect("REDACT_SAMPLE points to an image");
        let analysis = crate::ocr::recognize_sample(std::path::Path::new(&path));
        println!("{:#?}\n{:#?}", analysis.lines, analysis.sensitive);
        assert!(!analysis.sensitive.is_empty());
    }

    #[test]
    fn utf16_offsets_survive_multibyte_prefixes() {
        let line = "Correo 📧: ana@ejemplo.es";
        let (_, range) = find_sensitive(line).remove(0);
        assert_eq!(line[..range.start].encode_utf16().count(), "Correo 📧: ".encode_utf16().count());
    }
}
