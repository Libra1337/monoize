use sha2::{Digest, Sha256};

pub fn weak_etag(body: &[u8]) -> String {
    format!("W/\"{}\"", hex::encode(Sha256::digest(body)))
}

pub fn if_none_match_matches(header: &str, current: &str) -> bool {
    let header = header.trim();
    if header == "*" {
        return true;
    }
    if header.contains('*') || header.is_empty() {
        return false;
    }
    let Some(current_opaque) = parse_tag(current) else {
        return false;
    };
    let mut matched = false;
    for part in header.split(',') {
        let Some(candidate) = parse_tag(part.trim()) else {
            return false;
        };
        if candidate == current_opaque {
            matched = true;
        }
    }
    matched
}

fn parse_tag(value: &str) -> Option<&str> {
    let value = value.strip_prefix("W/").unwrap_or(value);
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }
    let opaque = &value[1..value.len() - 1];
    if opaque
        .bytes()
        .any(|byte| byte == b'"' || byte < 0x21 || byte == 0x7f)
    {
        return None;
    }
    Some(opaque)
}
