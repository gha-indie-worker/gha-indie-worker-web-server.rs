#![forbid(unsafe_code)]
//! The stateless HTTP avenue: `GHA_INDIE_WORKER_API_URL`.
//!
//! No client is wired in yet — the pages read fixtures — so what lives here is the part that would
//! be wrong in a subtle way if it were written inline at each call site: building a URL from a
//! configured base and a path without ever letting the path escape the base.

/// A configured API base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpTransport {
    pub base: String,
}

impl HttpTransport {
    #[must_use]
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }

    /// Join a path onto the base.
    ///
    /// Returns `None` for a path that is not rooted, that contains a `..` segment, or that is
    /// absolute in the scheme sense — all three are ways a caller-supplied identifier turns an
    /// internal call into a request to somewhere else entirely.
    #[must_use]
    pub fn url(&self, path: &str) -> Option<String> {
        if !path.starts_with('/') || path.starts_with("//") {
            return None;
        }
        if path.contains("://") || path.split('/').any(|segment| segment == "..") {
            return None;
        }
        Some(format!("{}{path}", self.base.trim_end_matches('/')))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_cannot_escape_the_configured_base() {
        let transport = HttpTransport::new("https://api.indiebuild.dev/");
        assert_eq!(
            transport.url("/v1/runs").as_deref(),
            Some("https://api.indiebuild.dev/v1/runs")
        );
        for bad in [
            "v1/runs",
            "//evil.test/v1",
            "https://evil.test",
            "/v1/../../admin",
            "/v1/..//x",
        ] {
            assert_eq!(transport.url(bad), None, "path {bad}");
        }
    }
}
