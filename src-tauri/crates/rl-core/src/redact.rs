//! Conservative diagnostic scrubbing for URLs and legacy credential strings.
pub fn redact(input: &str) -> String {
    let clean: String = input
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    clean
        .split('\n')
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("authorization:") || lower.contains("cookie:") {
                return "[redacted credentials]".into();
            }
            line.split_inclusive(char::is_whitespace)
                .map(|piece| {
                    let mut value = piece.to_string();
                    let lower = value.to_ascii_lowercase();
                    if let Some(start) = lower.find("https://").or_else(|| lower.find("http://")) {
                        let end = value.trim_end().len();
                        if let Ok(mut url) = reqwest::Url::parse(&value[start..end]) {
                            let _ = url.set_username("");
                            let _ = url.set_password(None);
                            if url.query().is_some() {
                                url.set_query(Some("redacted"));
                            }
                            if url.fragment().is_some() {
                                url.set_fragment(Some("redacted"));
                            }
                            value.replace_range(start..end, url.as_str());
                        }
                    }
                    let lower = value.to_ascii_lowercase();
                    for prefix in [
                        "ghp_",
                        "gho_",
                        "ghu_",
                        "ghs_",
                        "ghr_",
                        "github_pat_",
                        "token=",
                        "password=",
                        "secret=",
                        "signature=",
                    ] {
                        if let Some(start) = lower.find(prefix) {
                            let end = value.trim_end().len();
                            value.replace_range(start..end, "[redacted]");
                            break;
                        }
                    }
                    value
                })
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uppercase_url_schemes_do_not_leak_credentials() {
        let output = redact("GET HTTPS://user:private-pass@github.com/repo?key=private-query");
        assert!(!output.contains("private-pass"));
        assert!(!output.contains("private-query"));
    }
    #[test]
    fn removes_credentials_signed_queries_and_control_characters() {
        let output = redact(
            "GET https://release-assets.githubusercontent.com/file?sig=secret&token=hidden\nAuthorization: Bearer private-value\ntoken=legacy-value ghp_abcdefghijklmnopqrstuvwxyz123456\r\x1b[31m",
        );
        for secret in [
            "secret",
            "hidden",
            "private-value",
            "legacy-value",
            "abcdefghijklmnopqrstuvwxyz123456",
            "\x1b",
        ] {
            assert!(!output.contains(secret), "leaked {secret}");
        }
        assert!(output.contains("release-assets.githubusercontent.com"));
        assert_eq!(
            redact("Build installed successfully"),
            "Build installed successfully"
        );
    }
}
