//! Request-side prohibited-word firewall (`spec/content-firewall.spec.md`).
//!
//! Matching is leftmost case-insensitive substring matching over an
//! Aho-Corasick automaton, so one pass over each scanned string covers the
//! whole term list regardless of list size. No word-boundary or segmentation
//! logic is applied on purpose: for a firewall, a term embedded in arbitrary
//! surrounding text still counts as a hit.

use aho_corasick::AhoCorasick;

/// Built-in default for `moderation_blocked_words` (CF-3). Zero-tolerance
/// NSFW / child-sexual-abuse terms, bilingual, kept deliberately small;
/// operators extend the list through the admin settings page.
pub const DEFAULT_BLOCKED_WORDS: &str = concat!(
    "porn\n",
    "child erotica\n",
    "child nude\n",
    "csam\n",
    "csem\n",
    "sexualized minors\n",
    "underage nude\n",
    "色情\n",
    "黄片\n",
    "成人片\n",
    "儿童裸照\n",
    "儿童裸体\n",
    "未成年裸照\n",
    "未成年裸体\n",
    "嫖宿幼女\n",
    "强奸幼女\n",
    "猥亵儿童\n",
    "裸聊\n",
);

#[derive(Debug, Clone)]
pub struct ContentFirewall {
    /// Canonicalized terms, parallel to the automaton's pattern ids.
    terms: Vec<String>,
    matcher: AhoCorasick,
}

impl ContentFirewall {
    /// Compiles the raw newline-separated term list (CF-4, CF-5). Returns
    /// `None` when no term survives canonicalization, which disables scanning.
    pub fn compile(raw_blocked_words: &str) -> Option<Self> {
        let mut terms: Vec<String> = Vec::new();
        for line in raw_blocked_words.split('\n') {
            let term = line.trim().to_lowercase();
            if term.is_empty() || terms.contains(&term) {
                continue;
            }
            terms.push(term);
        }
        if terms.is_empty() {
            return None;
        }
        let matcher = AhoCorasick::new(&terms).expect("aho-corasick builds from valid strings");
        Some(Self { terms, matcher })
    }

    /// Returns the canonicalized term contained in `text`, if any (CF-5).
    pub fn find_blocked_term(&self, text: &str) -> Option<&str> {
        if text.is_empty() {
            return None;
        }
        let haystack = text.to_lowercase();
        self.matcher
            .find(&haystack)
            .map(|match_| self.terms[match_.pattern().as_usize()].as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_lists_compile_to_none() {
        assert!(ContentFirewall::compile("").is_none());
        assert!(ContentFirewall::compile(" \n\t\n  \n").is_none());
    }

    #[test]
    fn canonicalization_trims_lowercases_and_dedupes() {
        let firewall =
            ContentFirewall::compile("  Porn \nporn\nPORN\n\n儿童色情\n").expect("compiles");
        assert_eq!(firewall.find_blocked_term("clean text"), None);
        assert_eq!(firewall.find_blocked_term("some PORN here"), Some("porn"));
        assert_eq!(firewall.find_blocked_term("包含儿童色情内容"), Some("儿童色情"));
    }

    /// CF-5: terms match inside surrounding text without word boundaries, and
    /// case folding is Unicode-aware (simple lowercase folding: `Σ` folds to
    /// `σ`, but `ß` does not fold to `ss`).
    #[test]
    fn substring_matching_is_case_insensitive_and_unicode_aware() {
        let firewall = ContentFirewall::compile("σοφια\n").expect("compiles");
        assert_eq!(firewall.find_blocked_term("read ΣΟΦΙΑ now"), Some("σοφια"));
    }

    /// The leftmost match wins when several terms are present, which keeps the
    /// reported term deterministic.
    #[test]
    fn leftmost_match_is_reported() {
        let firewall = ContentFirewall::compile("beta\nalpha\n").expect("compiles");
        assert_eq!(firewall.find_blocked_term("xxx alpha beta"), Some("alpha"));
    }

    #[test]
    fn default_list_compiles_and_blocks_its_own_terms() {
        let firewall = ContentFirewall::compile(DEFAULT_BLOCKED_WORDS).expect("compiles");
        for term in DEFAULT_BLOCKED_WORDS.split('\n').filter(|t| !t.is_empty()) {
            assert_eq!(firewall.find_blocked_term(term), Some(term));
        }
        assert_eq!(firewall.find_blocked_term("hello world"), None);
        // Substring coverage: longer variants hit through their stem.
        assert_eq!(firewall.find_blocked_term("teen porn"), Some("porn"));
    }
}
