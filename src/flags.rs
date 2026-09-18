#![forbid(unsafe_code)]

//! Fail-closed argv and environment resolution through flags-2-env.
//!
//! The `.cli-flags.toml` contract is embedded at compile time and materialized
//! to a temporary file for the duration of one resolve, so a deployed binary
//! does not depend on its working directory containing the contract (the
//! runtime container image ships only the binary, sops and the entrypoint).
//!
//! Nothing in this module writes the process environment: the resolved values
//! are returned as an `EnvMap` snapshot. See `src/env_map.rs`.

use std::io::Write;

use crate::env_map::{merge_env, EnvMap};
use flags2env::BundledFlags2Env;
use tempfile::NamedTempFile;

const CONTRACT: &str = include_str!("../.cli-flags.toml");

pub fn resolve() -> Result<EnvMap, String> {
    resolve_from(&std::env::args().collect::<Vec<_>>(), std::env::vars())
}

pub fn resolve_from(
    argv: &[String],
    environment: impl IntoIterator<Item = (String, String)>,
) -> Result<EnvMap, String> {
    // Snapshot the environment once; it is both an input to flags-2-env and the
    // base the resolved values are overlaid onto, so env keys that are NOT
    // declared in .cli-flags.toml still reach the application. That is what
    // carries secret-only material such as GHA_INDIE_WORKER_DATABASE_URL,
    // which is deliberately not a public flag (see .cli-flags.toml).
    let environment: EnvMap = environment.into_iter().collect();
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

    // Errors name the option but never echo the value that failed.
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

    // Precedence, lowest to highest: contract dotenv, process environment,
    // dotenv overrides, flags actually given on the command line.
    let mut raw = parsed.dotenv;
    raw.extend(environment.clone());
    raw.extend(parsed.dotenv_overrides);
    raw.extend(parsed.provided_flags);
    let typed = parser
        .coerce::<serde_json::Map<String, serde_json::Value>, _>(&raw, Some(path))
        .map_err(|error| format!("flags-2-env typed configuration failed: {error}"))?;
    let declared: EnvMap = typed
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| scalar_string(&name, value).map(|value| (name, value)))
        .collect::<Result<EnvMap, String>>()?;
    // Declared/typed values (CLI overrides, contract defaults) win over the
    // raw environment snapshot; undeclared environment keys survive.
    Ok(merge_env(environment, declared))
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
    use crate::env_map::value;

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
    fn cli_overrides_merge_into_map_without_mutating_process_env() {
        let before = std::env::var_os("GHA_INDIE_WORKER_WEB_BIND");
        let env = resolve_from(
            &["svc".to_owned(), "--bind".to_owned(), "127.0.0.1:19001".to_owned()],
            std::iter::empty(),
        )
        .expect("valid flags");
        assert_eq!(value(&env, "GHA_INDIE_WORKER_WEB_BIND"), Some("127.0.0.1:19001"));
        assert_eq!(std::env::var_os("GHA_INDIE_WORKER_WEB_BIND"), before);
    }

    #[test]
    fn parse_failure_does_not_mutate_process_environment() {
        let before = std::env::var_os("ENV_MAP_PROBE");
        assert!(resolve_from(
            &["svc".to_owned(), "--this-flag-is-not-declared".to_owned()],
            [("ENV_MAP_PROBE".to_owned(), "keep".to_owned())],
        )
        .is_err());
        assert_eq!(std::env::var_os("ENV_MAP_PROBE"), before);
    }

    #[test]
    fn undeclared_secret_environment_keys_survive_resolution() {
        // GHA_INDIE_WORKER_DATABASE_URL is deliberately not a public flag, so it
        // is absent from .cli-flags.toml and from the generated runtime. It must
        // still reach the application through the environment snapshot.
        let env = resolve_from(
            &["svc".to_owned()],
            [(
                "GHA_INDIE_WORKER_DATABASE_URL".to_owned(),
                "postgres://synthetic.invalid/db".to_owned(),
            )],
        )
        .expect("valid flags");
        assert_eq!(
            value(&env, "GHA_INDIE_WORKER_DATABASE_URL"),
            Some("postgres://synthetic.invalid/db")
        );
    }

    #[test]
    fn source_does_not_mutate_process_environment() {
        const SRC: &str = include_str!("flags.rs");
        let production = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
        assert!(!production.contains("std::env::set_var"));
        assert!(!production.contains("env::set_var"));
    }
}
