const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
const REDACTED_URL_VALUE: &str = "<redacted>";
const SENSITIVE_URL_QUERY_KEYS: &[&str] = &[
    "access_token",
    "api_key",
    "client_secret",
    "id_token",
    "key",
    "refresh_token",
    "token",
];

pub fn redact_known_secret(input: &str, secret: &str) -> String {
    if secret.is_empty() {
        return input.to_string();
    }
    input.replace(secret, REDACTED_SECRET)
}

pub fn sanitize_url_for_display(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_fragment(None);

            let query_pairs = url
                .query_pairs()
                .map(|(key, value)| {
                    let key = key.into_owned();
                    let value = value.into_owned();
                    if SENSITIVE_URL_QUERY_KEYS
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(&key))
                    {
                        (key, REDACTED_URL_VALUE.to_string())
                    } else {
                        (key, value)
                    }
                })
                .collect::<Vec<_>>();

            if query_pairs.is_empty() {
                url.set_query(None);
            } else {
                let redacted_query = query_pairs
                    .into_iter()
                    .fold(
                        url::form_urlencoded::Serializer::new(String::new()),
                        |mut serializer, (key, value)| {
                            serializer.append_pair(&key, &value);
                            serializer
                        },
                    )
                    .finish();
                url.set_query(Some(&redacted_query));
            }

            url.to_string()
        }
        Err(_) => "<invalid-url>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{redact_known_secret, sanitize_url_for_display};

    #[test]
    fn redacts_known_secret_from_error_body() {
        assert_eq!(
            redact_known_secret("invalid token sk-secret-value", "sk-secret-value"),
            "invalid token [REDACTED_SECRET]"
        );
    }

    #[test]
    fn sanitizes_sensitive_url_parts() {
        let rendered =
            sanitize_url_for_display("https://user:pass@example.com/models?api_key=abc&env=prod");
        assert_eq!(
            rendered,
            "https://example.com/models?api_key=%3Credacted%3E&env=prod"
        );
    }
}
