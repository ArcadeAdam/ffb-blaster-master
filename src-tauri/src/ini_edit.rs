//! Line-preserving INI reader/editor.
//!
//! Deliberately NOT a parse-then-serialize round trip. FFBBlaster.ini files must
//! survive editing byte-for-byte apart from the values we intentionally change:
//! comments, blank lines, key order, key letter-casing and CRLF endings all stay.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IniEntry {
    pub section: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub section: String,
    pub key: String,
    pub value: String,
}

fn is_header(trimmed: &str) -> bool {
    trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2
}

fn is_comment(trimmed: &str) -> bool {
    trimmed.starts_with(';') || trimmed.starts_with('#')
}

fn eq(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn logical_trim(line: &str) -> &str {
    line.trim().trim_start_matches('\u{feff}')
}

/// Mixed CRLF/LF files (or old-style bare CR files) are rejected before a
/// write. Re-joining those files with one separator would silently change
/// untouched lines, which is contrary to this editor's safety guarantee.
pub fn has_mixed_or_unsupported_line_endings(content: &str) -> bool {
    let bytes = content.as_bytes();
    let mut has_crlf = false;
    let mut has_lf = false;

    for (i, byte) in bytes.iter().enumerate() {
        match byte {
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => return true,
            b'\n' if i > 0 && bytes[i - 1] == b'\r' => has_crlf = true,
            b'\n' => has_lf = true,
            _ => {}
        }
    }

    has_crlf && has_lf
}

/// Split content into logical lines while remembering how to put it back together.
fn split_lines(content: &str) -> (Vec<String>, &'static str, bool) {
    let eol = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let ends_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    if ends_nl {
        lines.pop();
    }
    (lines, eol, ends_nl)
}

/// Read every key/value pair, tagged with the section it lives in.
pub fn parse(content: &str) -> Vec<IniEntry> {
    let (lines, _, _) = split_lines(content);
    let mut section = String::new();
    let mut out = Vec::new();
    for line in &lines {
        let t = logical_trim(line);
        if is_header(t) {
            section = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if t.is_empty() || is_comment(t) {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let mut value = v.to_string();
            // Strip a trailing inline comment for display purposes only.
            if let Some(pos) = value.find([';', '#']) {
                value = value[..pos].to_string();
            }
            out.push(IniEntry {
                section: section.clone(),
                key: k.trim().to_string(),
                value: value.trim().to_string(),
            });
        }
    }
    out
}

fn find_key(lines: &[String], section: &str, key: &str) -> Option<usize> {
    let mut cur = String::new();
    for (i, line) in lines.iter().enumerate() {
        let t = logical_trim(line);
        if is_header(t) {
            cur = t[1..t.len() - 1].to_string();
            continue;
        }
        if !eq(&cur, section) || t.is_empty() || is_comment(t) {
            continue;
        }
        if let Some((k, _)) = t.split_once('=') {
            if eq(k, key) {
                return Some(i);
            }
        }
    }
    None
}

/// Index at which a new key should be inserted for `section`.
/// `None` means the section does not exist in the file at all.
fn section_insert_at(lines: &[String], section: &str) -> Option<usize> {
    let mut start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = logical_trim(line);
        if is_header(t) {
            if start.is_some() {
                return Some(i); // next section header: insert just before it
            }
            if eq(&t[1..t.len() - 1], section) {
                start = Some(i);
            }
        }
    }
    start.map(|_| lines.len())
}

/// Apply `changes` to `content`, touching only the lines that need to change.
pub fn apply_changes(content: &str, changes: &[Change]) -> String {
    let (mut lines, eol, ends_nl) = split_lines(content);

    for ch in changes {
        match find_key(&lines, &ch.section, &ch.key) {
            Some(i) => {
                let raw = lines[i].clone();
                let (before_equals, after_equals) =
                    raw.split_once('=').unwrap_or((raw.as_str(), ""));
                let comment_at = after_equals.find([';', '#']).unwrap_or(after_equals.len());
                let value_part = &after_equals[..comment_at];
                let comment = &after_equals[comment_at..];

                let leading_len = value_part.len() - value_part.trim_start().len();
                let leading = &value_part[..leading_len];
                let value_after_leading = &value_part[leading_len..];
                let trailing_at = value_after_leading.trim_end().len();
                let trailing = &value_after_leading[trailing_at..];

                // Keep the original indentation, key spelling, spaces around
                // the value, and inline comment. Replace only the value.
                lines[i] = format!(
                    "{}={}{}{}{}",
                    before_equals,
                    leading,
                    ch.value.trim(),
                    trailing,
                    comment
                );
            }
            None => match section_insert_at(&lines, &ch.section) {
                Some(mut at) => {
                    while at > 0 && lines[at - 1].trim().is_empty() {
                        at -= 1;
                    }
                    lines.insert(at, format!("{}={}", ch.key, ch.value.trim()));
                }
                None => {
                    if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                        lines.push(String::new());
                    }
                    lines.push(format!("[{}]", ch.section));
                    lines.push(format!("{}={}", ch.key, ch.value.trim()));
                }
            },
        }
    }

    let mut out = lines.join(eol);
    if ends_nl {
        out.push_str(eol);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[SETTINGS]\r\nDeviceGUID=0300c3096e3400000400000000000000\r\nMinForce=0\r\nMaxForce=100\r\ndisableInGameGui=0\r\nLampStart=LampStart\r\n";

    fn ch(k: &str, v: &str) -> Change {
        Change {
            section: "SETTINGS".into(),
            key: k.into(),
            value: v.into(),
        }
    }

    #[test]
    fn preserves_crlf_and_untouched_lines() {
        let out = apply_changes(SAMPLE, &[ch("MinForce", "12")]);
        assert!(out.contains("\r\nMinForce=12\r\n"));
        assert_eq!(out.matches("\r\n").count(), out.matches('\n').count());
        assert!(out.contains("LampStart=LampStart"));
        assert!(out.ends_with("\r\n"));
        assert_eq!(out.lines().count(), SAMPLE.lines().count());
    }

    #[test]
    fn matches_key_case_insensitively_but_keeps_original_spelling() {
        let out = apply_changes(SAMPLE, &[ch("DISABLEINGAMEGUI", "1")]);
        assert!(out.contains("disableInGameGui=1"));
        assert!(!out.contains("DISABLEINGAMEGUI"));
    }

    #[test]
    fn changes_nothing_when_value_is_identical() {
        let out = apply_changes(SAMPLE, &[ch("MaxForce", "100")]);
        assert_eq!(out, SAMPLE);
    }

    #[test]
    fn inserts_missing_key_at_end_of_section() {
        let out = apply_changes(SAMPLE, &[ch("StartDelay", "0")]);
        assert!(out.ends_with("StartDelay=0\r\n"));
    }

    #[test]
    fn keeps_inline_comments() {
        let src = "[SETTINGS]\nMinForce=0 ; leave low\n";
        let out = apply_changes(src, &[ch("MinForce", "20")]);
        assert_eq!(out, "[SETTINGS]\nMinForce=20 ; leave low\n");
    }

    #[test]
    fn changes_only_the_value_on_a_formatted_line() {
        let src = "[SETTINGS]\r\n  MinForce = 0   ; leave spacing alone\r\n";
        let out = apply_changes(src, &[ch("MinForce", "20")]);
        assert_eq!(
            out,
            "[SETTINGS]\r\n  MinForce = 20   ; leave spacing alone\r\n"
        );
    }

    #[test]
    fn understands_and_preserves_utf8_bom() {
        let src = "\u{feff}[SETTINGS]\r\nMinForce=0\r\n";
        let entries = parse(src);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].section, "SETTINGS");
        assert_eq!(
            apply_changes(src, &[ch("MinForce", "9")]),
            "\u{feff}[SETTINGS]\r\nMinForce=9\r\n"
        );
    }

    #[test]
    fn identifies_line_endings_that_are_unsafe_to_normalize() {
        assert!(!has_mixed_or_unsupported_line_endings("a=1\r\nb=2\r\n"));
        assert!(!has_mixed_or_unsupported_line_endings("a=1\nb=2\n"));
        assert!(has_mixed_or_unsupported_line_endings("a=1\r\nb=2\n"));
        assert!(has_mixed_or_unsupported_line_endings("a=1\rb=2\r"));
    }

    #[test]
    fn parses_entries() {
        let entries = parse(SAMPLE);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[1].key, "MinForce");
        assert_eq!(entries[1].section, "SETTINGS");
    }
}
