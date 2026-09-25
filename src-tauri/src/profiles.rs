//! Small, line-preserving reader/editor for TeknoParrot UserProfiles XML.
//!
//! TeknoParrot owns these files, so we deliberately avoid serializing the
//! whole document. Only the text inside the matching FFB Blaster FieldValue
//! element is replaced.

#[derive(Debug, Clone)]
pub struct ParsedProfile {
    pub name: String,
    pub exe: String,
    pub dir: String,
    pub ffb_supported: bool,
    pub ffb_enabled: bool,
}

fn decode_xml_text(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn tag_range(haystack: &str, tag: &str) -> Option<(usize, usize)> {
    let lower = haystack.to_ascii_lowercase();
    let open = format!("<{}>", tag.to_ascii_lowercase());
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find(&close)? + start;
    Some((start, end))
}

fn tag_text(haystack: &str, tag: &str) -> Option<String> {
    let (start, end) = tag_range(haystack, tag)?;
    Some(decode_xml_text(haystack[start..end].trim()))
}

fn ffb_block(content: &str) -> Option<(usize, usize)> {
    let lower = content.to_ascii_lowercase();
    let open = "<fieldinformation>";
    let close = "</fieldinformation>";
    let mut cursor = 0;
    while let Some(relative_start) = lower[cursor..].find(open) {
        let start = cursor + relative_start;
        let Some(relative_end) = lower[start..].find(close) else {
            return None;
        };
        let end = start + relative_end + close.len();
        let block = &content[start..end];
        let category = tag_text(block, "CategoryName").unwrap_or_default();
        let field = tag_text(block, "FieldName").unwrap_or_default();
        if category.eq_ignore_ascii_case("FFB Blaster") && field.eq_ignore_ascii_case("Enable") {
            return Some((start, end));
        }
        cursor = end;
    }
    None
}

pub fn parse_profile(content: &str, fallback_name: &str) -> ParsedProfile {
    let name = tag_text(content, "GameNameInternal")
        .filter(|s| !s.is_empty())
        .or_else(|| tag_text(content, "ProfileName").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| fallback_name.to_string());
    let exe = tag_text(content, "GamePath").unwrap_or_default();
    let dir = exe
        .rfind(['\\', '/'])
        .map(|at| exe[..at].to_string())
        .unwrap_or_default();
    let value =
        ffb_block(content).and_then(|(start, end)| tag_text(&content[start..end], "FieldValue"));

    ParsedProfile {
        name,
        exe,
        dir,
        ffb_supported: value.is_some(),
        ffb_enabled: value
            .as_deref()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    }
}

/// Returns `None` when the profile does not support FFB Blaster.
pub fn set_ffb_enabled(content: &str, enabled: bool) -> Option<String> {
    let (block_start, block_end) = ffb_block(content)?;
    let block = &content[block_start..block_end];
    let (relative_start, relative_end) = tag_range(block, "FieldValue")?;
    let start = block_start + relative_start;
    let end = block_start + relative_end;
    let old = &content[start..end];
    let leading_len = old.len() - old.trim_start().len();
    let trailing_at = old.trim_end().len();
    let replacement = format!(
        "{}{}{}",
        &old[..leading_len],
        if enabled { "1" } else { "0" },
        &old[trailing_at..]
    );

    let mut updated = String::with_capacity(content.len() - old.len() + replacement.len());
    updated.push_str(&content[..start]);
    updated.push_str(&replacement);
    updated.push_str(&content[end..]);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<GameProfile>\r\n  <ProfileName>RawName</ProfileName>\r\n  <GameNameInternal>Cruis&apos;n &amp; Test</GameNameInternal>\r\n  <GamePath>R:\\Arcade\\TeknoParrot\\GAME\\game.exe</GamePath>\r\n  <ConfigValues>\r\n    <FieldInformation>\r\n      <CategoryName>General</CategoryName>\r\n      <FieldName>Windowed</FieldName>\r\n      <FieldValue>0</FieldValue>\r\n    </FieldInformation>\r\n    <FieldInformation>\r\n      <CategoryName>FFB Blaster</CategoryName>\r\n      <FieldName>Enable</FieldName>\r\n      <FieldValue>  0  </FieldValue>\r\n    </FieldInformation>\r\n  </ConfigValues>\r\n</GameProfile>\r\n";

    #[test]
    fn reads_profile_metadata_and_ffb_state() {
        let parsed = parse_profile(PROFILE, "fallback");
        assert_eq!(parsed.name, "Cruis'n & Test");
        assert_eq!(parsed.exe, "R:\\Arcade\\TeknoParrot\\GAME\\game.exe");
        assert_eq!(parsed.dir, "R:\\Arcade\\TeknoParrot\\GAME");
        assert!(parsed.ffb_supported);
        assert!(!parsed.ffb_enabled);
    }

    #[test]
    fn changes_only_the_ffb_value() {
        let updated = set_ffb_enabled(PROFILE, true).unwrap();
        assert_eq!(updated.matches("<FieldValue>0</FieldValue>").count(), 1);
        assert!(updated.contains("<FieldValue>  1  </FieldValue>"));
        assert_eq!(
            updated.matches("\r\n").count(),
            PROFILE.matches("\r\n").count()
        );
        assert!(updated.starts_with('\u{feff}'));
    }

    #[test]
    fn unsupported_profile_is_not_modified() {
        let plain = "<GameProfile><ProfileName>No FFB</ProfileName></GameProfile>";
        assert!(set_ffb_enabled(plain, true).is_none());
        assert!(!parse_profile(plain, "fallback").ffb_supported);
    }
}
