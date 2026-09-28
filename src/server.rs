#![forbid(unsafe_code)]

use crate::config::WebConfig;
use crate::error::WebError;
use crate::pages;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendCapability {
    DirectReadOnlyDatabase,
    StatelessHttp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPlan {
    pub bind: String,
    pub capabilities: Vec<BackendCapability>,
}

pub fn startup_plan(config: &WebConfig) -> Result<StartupPlan, WebError> {
    let bind = non_empty("GHA_INDIE_WORKER_WEB_BIND", &config.bind)?;
    let optional_capabilities = [
        config.database_url.as_ref().map(|value| {
            (
                BackendCapability::DirectReadOnlyDatabase,
                "GHA_INDIE_WORKER_DATABASE_URL",
                value,
            )
        }),
        config.api_http_base.as_ref().map(|value| {
            (
                BackendCapability::StatelessHttp,
                "GHA_INDIE_WORKER_API_HTTP_BASE",
                value,
            )
        }),
    ];
    let capabilities = optional_capabilities
        .into_iter()
        .flatten()
        .map(|(capability, field, value)| non_empty(field, value).map(|_| capability))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(StartupPlan { bind, capabilities })
}

fn non_empty(field: &'static str, value: &str) -> Result<String, WebError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(WebError::InvalidConfiguration(field));
    }
    Ok(value.to_owned())
}

pub fn run(config: &WebConfig) -> Result<(), WebError> {
    let plan = startup_plan(config)?;
    println!("web bind {}", plan.bind);
    println!("{}", pages::home::markup());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{startup_plan, BackendCapability, StartupPlan};
    use crate::{config::WebConfig, error::WebError};

    #[test]
    fn startup_plan_derives_capabilities_without_retaining_the_database_url() {
        let config = WebConfig {
            bind: " 127.0.0.1:8081 ".into(),
            api_http_base: Some("http://api:8080".into()),
            database_url: Some("postgres://sensitive-value".into()),
        };

        let plan = startup_plan(&config).expect("valid web startup plan");

        assert_eq!(
            plan,
            StartupPlan {
                bind: "127.0.0.1:8081".into(),
                capabilities: vec![
                    BackendCapability::DirectReadOnlyDatabase,
                    BackendCapability::StatelessHttp,
                ],
            }
        );
        assert!(!format!("{plan:?}").contains("sensitive-value"));
    }

    #[test]
    fn startup_plan_rejects_blank_optional_configuration() {
        let error = startup_plan(&WebConfig {
            bind: "127.0.0.1:8081".into(),
            api_http_base: Some("  ".into()),
            database_url: None,
        })
        .expect_err("blank API base must fail closed");

        assert!(matches!(
            error,
            WebError::InvalidConfiguration("GHA_INDIE_WORKER_API_HTTP_BASE")
        ));
    }
}
