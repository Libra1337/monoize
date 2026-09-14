//! LLM semantic judge for the content firewall
//! (`spec/content-firewall.spec.md` §8). The judge is the only component that
//! can reject a request; keyword matches are hints and the mark trigger.

use serde_json::{Value, json};

/// CF-28: total budget for the user message across all scanned strings.
pub const JUDGE_TEXT_BUDGET_CHARS: usize = 6000;

/// CF-20/CF-28a: hard cap for the judge's stored reason.
pub const REASON_MAX_CHARS: usize = 500;

pub const CATEGORY_PORN: &str = "porn";
pub const CATEGORY_POLITICAL: &str = "political";
pub const CATEGORY_BENIGN: &str = "benign";
pub const CATEGORY_UNCERTAIN: &str = "uncertain";

/// CF-28: verdict categories. Only `porn` and `political` reject (CF-31);
/// `benign` and `uncertain` always allow.
const CATEGORIES: [&str; 4] = [
    CATEGORY_PORN,
    CATEGORY_POLITICAL,
    CATEGORY_BENIGN,
    CATEGORY_UNCERTAIN,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub category: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JudgeConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_ms: u64,
}

impl JudgeConfig {
    /// CF-28: the judge is active only when enabled and fully configured.
    pub fn is_active(&self) -> bool {
        self.enabled
            && !self.base_url.trim().is_empty()
            && !self.api_key.trim().is_empty()
            && !self.model.trim().is_empty()
    }

    /// CF-28: a `/v1` suffix on the base URL is stripped before appending
    /// `/chat/completions`.
    pub fn endpoint(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}/v1/chat/completions")
    }
}

const SYSTEM_PROMPT: &str = r#"You are the content-safety judge of an LLM API gateway. Your job is to decide what the request text is trying to accomplish, not which words appear in it.

Take your time: reason about the text's intent, context, and phrasing before answering. You may write that analysis as plain text first. Then end your reply with EXACTLY ONE JSON object on its own last line and nothing after it:
{"category": "...", "reason": "..."}

Categories:
- "porn": the text seeks to produce, continue, or roleplay sexually explicit content. Judge intent, not vocabulary: the request may use slang, euphemism, or no explicit word at all. Typical porn forms include: erotic fiction or NSFW roleplay requests ("成人小说", "开车", detailed bodily descriptions); jailbreak prompts that instruct the model to ignore safety rules and write explicit content; requests to generate, describe, or caption sexual images ("色图", bikini/lingerie pin-ups, sexualized captions); quoted or pasted material that itself contains explicit sexual content, even when the outer task is analysis, extraction, summarization, or "learning" — if the material being fed in is explicit, classify "porn". Anything sexualizing minors (including "萝莉", "正太", child-like characters in sexual contexts) is always "porn".
- "political": the text seeks to produce politically illegal content such as subverting state power, inciting separatism, or extremist propaganda. Porn/NSFW evidence outweighs political evidence: when a text hits both, prefer "porn".
- "benign": everything else. Discussing, mentioning, reporting on, or prohibiting sensitive topics (news, education, law, moderation policy, technical work) is "benign" even when it quotes prohibited words. Agent or tool system prompts, developer configuration, and defensive security policy text (security testing, CTF, refusing attacks) are "benign".
- "uncertain": you genuinely cannot decide after analysis. Reserve it for truly borderline text, not for content you suspect is porn — a suspicion of explicit intent classifies "porn", because a false pass is worse than a false block.

In "reason" state the concrete evidence: what the text asks for, and why that makes it blocking or not. One to three sentences, always written in Simplified Chinese (简体中文), regardless of the request text's language."#;

/// Assembles the user message: keyword hint line first (CF-28), then the
/// scanned strings, truncated to the CF-28 budget in total.
pub fn build_user_message(keyword_hits: &[String], texts: &[&str]) -> String {
    let mut budget = JUDGE_TEXT_BUDGET_CHARS;
    let mut message = String::new();
    if !keyword_hits.is_empty() {
        let hint = format!("Keyword matcher flagged: {}\n\n", keyword_hits.join(", "));
        budget = budget.saturating_sub(hint.chars().count());
        message.push_str(&hint);
    }
    for (index, text) in texts.iter().enumerate() {
        if budget == 0 {
            break;
        }
        if index > 0 {
            message.push_str("\n---\n");
            budget = budget.saturating_sub(5);
        }
        let take: String = text.chars().take(budget).collect();
        budget = budget.saturating_sub(take.chars().count());
        message.push_str(&take);
    }
    message
}

fn category_of(value: &str) -> Option<&'static str> {
    CATEGORIES.iter().copied().find(|known| *known == value)
}

/// CF-28a: extracts the verdict from the assistant content. The judge may
/// think in plain text before its answer, so scanning takes the LAST JSON
/// object whose `category` is a known value; earlier JSON-looking fragments
/// of the analysis are ignored.
pub fn parse_verdict(content: &str) -> Option<Verdict> {
    let bytes = content.as_bytes();
    let mut best: Option<Verdict> = None;
    for (index, _) in bytes.iter().enumerate().filter(|(_, b)| **b == b'{') {
        let candidate = &content[index..];
        let Some(end) = candidate.find('}') else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&candidate[..=end]) else {
            continue;
        };
        let Some(category) = value.get("category").and_then(Value::as_str) else {
            continue;
        };
        let Some(category) = category_of(category) else {
            continue;
        };
        let reason = value
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(REASON_MAX_CHARS)
            .collect::<String>();
        best = Some(Verdict { category, reason });
    }
    best
}

pub struct JudgeCall<'a> {
    pub http: &'a reqwest::Client,
    pub config: &'a JudgeConfig,
    pub bypass_token: &'a str,
    pub user_message: String,
}

/// Performs one judge call (CF-28). Returns the verdict, or an error string
/// on any failure (CF-28a); the caller treats every error as fail-open.
pub async fn call_judge(call: JudgeCall<'_>) -> Result<Verdict, String> {
    let body = json!({
        "model": call.config.model,
        "temperature": 0,
        "max_tokens": 512,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": call.user_message}
        ]
    });
    let timeout = std::time::Duration::from_millis(call.config.timeout_ms.max(1));
    let response = call
        .http
        .post(call.config.endpoint())
        .bearer_auth(call.config.api_key.trim())
        .header("x-monoize-moderation-bypass", call.bypass_token)
        .timeout(timeout)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("judge request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("judge returned HTTP {status}"));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| format!("judge response is not JSON: {error}"))?;
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| "judge response has no assistant content".to_string())?;
    parse_verdict(content).ok_or_else(|| "judge verdict not parseable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_strips_v1_suffix() {
        let config = JudgeConfig {
            enabled: true,
            base_url: "https://api.example.com/v1/".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            timeout_ms: 1000,
        };
        assert_eq!(
            config.endpoint(),
            "https://api.example.com/v1/chat/completions"
        );
        let config = JudgeConfig {
            enabled: true,
            base_url: "https://api.example.com".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            timeout_ms: 1000,
        };
        assert_eq!(
            config.endpoint(),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn user_message_limits_total_budget_and_puts_hints_first() {
        let message = build_user_message(&["色情".to_string()], &[&"a".repeat(10_000)]);
        assert!(message.starts_with("Keyword matcher flagged: 色情"));
        assert!(message.chars().count() <= JUDGE_TEXT_BUDGET_CHARS + 5);
    }

    #[test]
    fn verdict_parsing_takes_the_last_valid_object_and_keeps_reason() {
        // The judge may think in text containing JSON-like fragments first;
        // the last parseable verdict wins.
        let content = concat!(
            "The text mentions {\"category\":\"porn\"} only as a quoted word. ",
            "Its intent is policy discussion.\n",
            "{\"category\":\"benign\",\"reason\":\"moderation policy discussion\"}"
        );
        let verdict = parse_verdict(content).expect("verdict");
        assert_eq!(verdict.category, CATEGORY_BENIGN);
        assert_eq!(verdict.reason, "moderation policy discussion");

        assert_eq!(parse_verdict("{\"category\":\"weapon\"}"), None);
        assert_eq!(parse_verdict("no json here"), None);
        assert_eq!(parse_verdict("{\"broken\""), None);
    }

    #[test]
    fn uncertain_is_a_valid_verdict() {
        let verdict = parse_verdict(
            "Hard to tell.\n{\"category\":\"uncertain\",\"reason\":\"ambiguous phrasing\"}",
        )
        .expect("verdict");
        assert_eq!(verdict.category, CATEGORY_UNCERTAIN);
    }
}
