//! LLM semantic judge for the content firewall
//! (`spec/content-firewall.spec.md` §8). The judge is the only component that
//! can reject a request; keyword matches are hints and the mark trigger.

use serde_json::{Value, json};

/// CF-28: total budget for the user message across all scanned strings.
pub const JUDGE_TEXT_BUDGET_CHARS: usize = 6000;

pub const CATEGORY_PORN: &str = "porn";
pub const CATEGORY_POLITICAL: &str = "political";
pub const CATEGORY_BENIGN: &str = "benign";

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

const SYSTEM_PROMPT: &str = r#"You are the content-safety classifier of an LLM API gateway. Classify the request text into exactly one category and answer with a single JSON object and nothing else: {"category":"porn"} or {"category":"political"} or {"category":"benign"}.
- "porn": the text seeks to produce, continue, or roleplay sexually explicit content. Anything sexualizing minors is always "porn".
- "political": the text seeks to produce politically illegal content such as subverting state power, inciting separatism, or extremist propaganda.
- "benign": everything else. Discussing, mentioning, reporting on, prohibiting, or studying these topics (news, education, law, moderation policy, technical work) is "benign", even when it quotes prohibited words.
Judge what the text is trying to accomplish, not which words appear in it."#;

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

/// Extracts the category from the assistant content: the first JSON object's
/// `category` field, validated against the three known values (CF-28a).
pub fn parse_verdict(content: &str) -> Option<&'static str> {
    let start = content.find('{')?;
    let end = content[start..].find('}')? + start;
    let value: Value = serde_json::from_str(&content[start..=end]).ok()?;
    let category = value.get("category")?.as_str()?;
    match category {
        CATEGORY_PORN => Some(CATEGORY_PORN),
        CATEGORY_POLITICAL => Some(CATEGORY_POLITICAL),
        CATEGORY_BENIGN => Some(CATEGORY_BENIGN),
        _ => None,
    }
}

pub struct JudgeCall<'a> {
    pub http: &'a reqwest::Client,
    pub config: &'a JudgeConfig,
    pub bypass_token: &'a str,
    pub user_message: String,
}

/// Performs one judge call (CF-28). Returns the category, or an error string
/// on any failure (CF-28a); the caller treats every error as fail-open.
pub async fn call_judge(call: JudgeCall<'_>) -> Result<&'static str, String> {
    let body = json!({
        "model": call.config.model,
        "temperature": 0,
        "max_tokens": 64,
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
    parse_verdict(content).ok_or_else(|| format!("judge verdict not parseable: {content}"))
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
        assert_eq!(config.endpoint(), "https://api.example.com/v1/chat/completions");
        let config = JudgeConfig {
            enabled: true,
            base_url: "https://api.example.com".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            timeout_ms: 1000,
        };
        assert_eq!(config.endpoint(), "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn user_message_limits_total_budget_and_puts_hints_first() {
        let message = build_user_message(&["色情".to_string()], &[&"a".repeat(10_000)]);
        assert!(message.starts_with("Keyword matcher flagged: 色情"));
        assert!(message.chars().count() <= JUDGE_TEXT_BUDGET_CHARS + 5);
    }

    #[test]
    fn verdict_parsing_accepts_json_with_surroundings_and_rejects_garbage() {
        assert_eq!(
            parse_verdict("Sure! {\"category\":\"porn\"} hope that helps"),
            Some(CATEGORY_PORN)
        );
        assert_eq!(parse_verdict("{\"category\":\"benign\"}"), Some(CATEGORY_BENIGN));
        assert_eq!(parse_verdict("{\"category\":\"weapon\"}"), None);
        assert_eq!(parse_verdict("no json here"), None);
        assert_eq!(parse_verdict("{\"broken\""), None);
    }
}
