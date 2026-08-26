use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPublicName {
    pub value: String,
    pub key: Vec<u8>,
}

pub fn canonicalize_public_name(raw: &str) -> Result<CanonicalPublicName, String> {
    let value = raw
        .trim_matches(char::is_whitespace)
        .nfc()
        .collect::<String>();
    if !(1..=64).contains(&value.chars().count()) {
        return Err("public name must contain 1 through 64 Unicode scalar values".to_string());
    }
    if value
        .as_bytes()
        .iter()
        .any(|byte| matches!(*byte, 0x00..=0x1f | 0x7f))
    {
        return Err("public name must not contain C0 or DEL control characters".to_string());
    }
    let key = value.as_bytes().to_vec();
    Ok(CanonicalPublicName { value, key })
}

#[cfg(test)]
mod tests {
    use super::canonicalize_public_name;

    #[test]
    fn canonicalizes_nfc_and_rejects_controls() {
        let name = canonicalize_public_name("  Cafe\u{301}  ").unwrap();
        assert_eq!(name.value, "Caf\u{e9}");
        assert_eq!(name.key, "Caf\u{e9}".as_bytes());
        assert!(canonicalize_public_name("bad\nname").is_err());
        assert!(canonicalize_public_name(&"a".repeat(65)).is_err());
    }
}
