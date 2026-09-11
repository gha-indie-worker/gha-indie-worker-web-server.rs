//! Fail-closed argv and environment resolution through flags-2-env.

use std::collections::BTreeMap;
use std::io::Write;

use flags2env::BundledFlags2Env;
use tempfile::NamedTempFile;

const CONTRACT: &str = include_str!("../.cli-flags.toml");
const ENV_ONLY_URLS: [&str; 2] = [
    "GHA_INDIE_WORKER_API_HTTP_BASE",
    "GHA_INDIE_WORKER_DATABASE_URL",
];

pub fn resolve() -> Result<BTreeMap<String, String>, String> {
    resolve_from(&std::env::args().collect::<Vec<_>>(), std::env::vars())
}

fn resolve_from(
    argv: &[String],
    environment: impl IntoIterator<Item = (String, String)>,
) -> Result<BTreeMap<String, String>, String> {
    let environment = environment.into_iter().collect::<BTreeMap<_, _>>();
    let mut contract = NamedTempFile::new()
        .map_err(|error| format!("cannot create embedded flags-2-env contract: {error}"))?;
    contract
        .write_all(CONTRACT.as_bytes())
        .map_err(|error| format!("cannot materialize embedded flags-2-env contract: {error}"))?;
    let path = contract
        .path()
        .to_str()
        .ok_or_else(|| "flags-2-env contract path is not valid UTF-8".to_owned())?;
    let parser = BundledFlags2Env::new();
    parser
        .audit_config(Some(path))
        .map_err(|error| format!("flags-2-env contract audit failed: {error}"))?;
    let parsed = parser
        .parse_structured(argv, Some(path))
        .map_err(|error| format!("flags-2-env parsing failed: {error}"))?;

    if !parsed.unknown_options.is_empty() {
        let names = parsed
            .unknown_options
            .iter()
            .map(|option| option.split('=').next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("unknown command-line option(s): {names}"));
    }
    if !parsed.errors.is_empty() {
        return Err(format!(
            "invalid command-line value(s): {}",
            parsed.errors.join("; ")
        ));
    }
    if !parsed.extras.is_empty() {
        return Err(format!(
            "unexpected positional argument(s): {}",
            parsed.extras.len()
        ));
    }

    // `.cli-flags.toml` owns public argv normalization, but plaintext dotenv is
    // not an application configuration source. Resolve declared public values
    // from the explicit process environment and actual argv overrides only.
    let mut raw = environment.clone();
    raw.extend(parsed.provided_flags);
    let typed = parser
        .coerce::<serde_json::Map<String, serde_json::Value>, _>(&raw, Some(path))
        .map_err(|error| format!("flags-2-env typed configuration failed: {error}"))?;
    let mut resolved = typed
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| scalar_string(&name, value).map(|value| (name, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    // Connection URLs may embed credentials. They deliberately stay outside
    // argv and are copied only from the process secret/configuration boundary.
    for name in ENV_ONLY_URLS {
        if let Some(value) = environment.get(name) {
            if !value.trim().is_empty() {
                resolved.insert(name.to_owned(), value.clone());
            }
        }
    }

    Ok(resolved)
}

fn scalar_string(name: &str, value: serde_json::Value) -> Result<String, String> {
    match value {
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Bool(value) => Ok(value.to_string()),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        _ => Err(format!(
            "flags-2-env returned a non-scalar value for {name}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_options_fail_closed_without_echoing_values() {
        let error = resolve_from(
            &[
                "server".to_owned(),
                "--definitely-unknown=do-not-echo".to_owned(),
            ],
            std::iter::empty(),
        )
        .expect_err("unknown option");
        assert!(error.contains("--definitely-unknown"));
        assert!(!error.contains("do-not-echo"));
    }

    #[test]
    fn connection_urls_are_environment_only() {
        let database = "postgres://worker:synthetic-credential@db.internal/app";
        let api = "https://service:synthetic-credential@api.internal";
        let resolved = resolve_from(
            &["server".to_owned()],
            [
                (ENV_ONLY_URLS[0].to_owned(), api.to_owned()),
                (ENV_ONLY_URLS[1].to_owned(), database.to_owned()),
            ],
        )
        .expect("environment-only connection URLs");
        assert_eq!(resolved.get(ENV_ONLY_URLS[0]).map(String::as_str), Some(api));
        assert_eq!(
            resolved.get(ENV_ONLY_URLS[1]).map(String::as_str),
            Some(database)
        );

        for (flag, secret) in [
            ("--gha-indie-worker-api-http-base", "synthetic-api-secret"),
            ("--gha-indie-worker-database-url", "synthetic-db-secret"),
        ] {
            let argument = format!("{flag}=https://user:{secret}@example.invalid");
            let error = resolve_from(&["server".to_owned(), argument], std::iter::empty())
                .expect_err("connection URL must not be accepted through argv");
            assert!(error.contains(flag));
            assert!(!error.contains(secret));
        }
    }

    #[test]
    fn plaintext_dotenv_is_not_a_runtime_source() {
        const SOURCE: &str = include_str!("flags.rs");
        let production = SOURCE.split("#[cfg(test)]").next().unwrap_or(SOURCE);
        assert!(!production.contains("parsed.dotenv"));
        assert!(!production.contains("dotenv_overrides"));
    }
}
