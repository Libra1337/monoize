pub fn wechat_signature_message(
    method: &str,
    canonical_url: &str,
    timestamp: &str,
    nonce: &str,
    body: &str,
) -> String {
    format!("{method}\n{canonical_url}\n{timestamp}\n{nonce}\n{body}\n")
}
