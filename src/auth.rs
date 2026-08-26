#![forbid(unsafe_code)]

use crate::error::WebError;

pub fn require_bearer(header: Option<&str>) -> Result<&str, WebError> {
    let value = header.ok_or(WebError::Unauthenticated)?;
    value.strip_prefix("Bearer ").filter(|t| !t.is_empty()).ok_or(WebError::Unauthenticated)
}

