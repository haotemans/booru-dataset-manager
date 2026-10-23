#![forbid(unsafe_code)]
//! Use-case orchestration. UI, SQL, HTTP and model details belong in adapters.
pub mod catalog;
pub mod ingest;
pub fn implementation_status() -> &'static str {
    "Danbooru ingestion, durable media, character hierarchy, text optimization, pHash/DINOv3 deduplication and basic export are implemented; advanced scheduling and remote backup remain pending."
}

pub fn validate_source_credentials(username: &str, api_key: &str) -> Result<(), &'static str> {
    if username.trim().is_empty() {
        return Err("请填写 Danbooru 用户名。");
    }
    if username.len() > 256 || username.chars().any(char::is_control) {
        return Err("用户名过长或包含无效字符。");
    }
    if api_key.trim().is_empty() {
        return Err("请填写 API Key。");
    }
    if api_key.len() > 1024
        || username.len() + api_key.len() > 1279
        || api_key.chars().any(char::is_whitespace)
        || api_key.chars().any(char::is_control)
    {
        return Err("API Key 格式无效，请检查复制内容。");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_credentials_require_a_valid_identity_and_key() {
        assert!(validate_source_credentials("artist_name", "synthetic-api-key").is_ok());
        for (name, key) in [
            ("", "key"),
            ("user\nother", "key"),
            ("user", ""),
            ("user", "key with spaces"),
            ("user", "key\0tail"),
        ] {
            assert!(validate_source_credentials(name, key).is_err());
        }
    }
}
