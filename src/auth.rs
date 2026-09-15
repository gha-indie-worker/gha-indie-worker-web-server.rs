#![forbid(unsafe_code)]

use crate::error::WebError;

pub fn require_bearer(header: Option<&str>) -> Result<&str, WebError> {
    let value = header.ok_or(WebError::Unauthenticated)?;
    value
        .strip_prefix("Bearer ")
        .filter(|t| !t.is_empty())
        .ok_or(WebError::Unauthenticated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_nonempty_canonical_bearer_values() {
        assert_eq!(
            require_bearer(Some("Bearer token")).expect("token"),
            "token"
        );
        assert!(matches!(
            require_bearer(None),
            Err(WebError::Unauthenticated)
        ));
        assert!(matches!(
            require_bearer(Some("Bearer ")),
            Err(WebError::Unauthenticated)
        ));
        assert!(matches!(
            require_bearer(Some("bearer token")),
            Err(WebError::Unauthenticated)
        ));
    }
}
