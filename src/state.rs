#![forbid(unsafe_code)]

use crate::config::WebConfig;

#[derive(Clone, Debug)]
pub struct AppState {
    pub config: WebConfig,
}

