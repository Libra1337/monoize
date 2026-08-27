use std::collections::BTreeMap;

pub fn canonical_alipay_parameters(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .filter(|(key, value)| {
            !value.is_empty() && key.as_str() != "sign" && key.as_str() != "sign_type"
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}
