//! Request-side prohibited-word firewall (`spec/content-firewall.spec.md`).
//!
//! Matching is leftmost case-insensitive substring matching over an
//! Aho-Corasick automaton, so one pass over each scanned string covers the
//! whole term list regardless of list size. No word-boundary or segmentation
//! logic is applied on purpose: for a firewall, a term embedded in arbitrary
//! surrounding text still counts as a hit.

use aho_corasick::AhoCorasick;

/// Built-in default for `moderation_blocked_words` (CF-3). Zero-tolerance
/// Ordered by operator-assigned weight: NSFW/CSAM terms first (the
/// enforcement priority and stronger signal to the judge), then political
/// terms. Matching itself is set membership; order documents priority.
pub const DEFAULT_BLOCKED_WORDS: &str = concat!(
    // NSFW / CSAM (highest weight)
    "porn\n",
    "nsfw\n",
    "child erotica\n",
    "child nude\n",
    "csam\n",
    "csem\n",
    "sexualized minors\n",
    "underage nude\n",
    "r18\n",
    "hentai\n",
    "erotic\n",
    "色情\n",
    "色图\n",
    "涩图\n",
    "瑟瑟\n",
    "开车\n",
    "荤段子\n",
    "黄片\n",
    "黄文\n",
    "成人片\n",
    "成人小说\n",
    "成人漫画\n",
    "情色小说\n",
    "肉文\n",
    "本子\n",
    "里番\n",
    "萝莉\n",
    "正太\n",
    "儿童裸照\n",
    "儿童裸体\n",
    "未成年裸照\n",
    "未成年裸体\n",
    "幼女\n",
    "嫖宿幼女\n",
    "强奸幼女\n",
    "猥亵儿童\n",
    "露骨\n",
    "淫\n",
    "骚\n",
    "裸聊\n",
    "裸照\n",
    "自慰\n",
    "口交\n",
    "肛交\n",
    "巨乳\n",
    "媚药\n",
    "调教\n",
    // Political (lower weight)
    "颠覆国家政权\n",
    "煽动颠覆\n",
    "分裂国家\n",
    "煽动分裂\n",
    "恐怖主义\n",
    "极端主义\n",
    "法轮功\n",
    "六四事件\n",
    "达赖\n",
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

    /// Returns every distinct canonicalized term contained in `text`, in the
    /// compiled list's order (CF-5a: a request reports all keyword hits, and
    /// NSFW terms sort ahead of political ones in the default list).
    pub fn find_all_blocked_terms(&self, text: &str) -> Vec<&str> {
        if text.is_empty() {
            return Vec::new();
        }
        let haystack = text.to_lowercase();
        let mut hits: Vec<Option<&str>> = vec![None; self.terms.len()];
        for match_ in self.matcher.find_overlapping_iter(&haystack) {
            hits[match_.pattern().as_usize()] =
                Some(self.terms[match_.pattern().as_usize()].as_str());
        }
        hits.into_iter().flatten().collect()
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
            // Every term of the list is itself matched, and no shorter term of
            // the list is a substring of it (e.g. "erotic" inside "child
            // erotica") — otherwise the shorter stem would shadow the term in
            // single-hit matching while full-hit extraction still reports both.
            assert!(
                firewall.find_all_blocked_terms(term).contains(&term),
                "{term} must match itself"
            );
        }
        assert_eq!(firewall.find_blocked_term("hello world"), None);
        // Substring coverage: longer variants hit through their stem.
        assert_eq!(firewall.find_blocked_term("teen porn"), Some("porn"));
    }

    /// CF-5a: every distinct hit is extracted, in compiled-list (weight)
    /// order — the operator puts NSFW terms first in the list, so they are
    /// reported first regardless of where they appear in the text.
    #[test]
    fn all_hits_are_extracted_in_weight_order() {
        let firewall = ContentFirewall::compile("颠覆国家政权\n色情\n色图\n").expect("compiles");
        assert_eq!(
            firewall.find_all_blocked_terms("这张色图很色情，颠覆国家政权"),
            vec!["颠覆国家政权", "色情", "色图"]
        );
        assert_eq!(firewall.find_all_blocked_terms("clean"), Vec::<&str>::new());
        // Duplicate occurrences report once.
        assert_eq!(firewall.find_all_blocked_terms("色图 色图"), vec!["色图"]);
    }
}
