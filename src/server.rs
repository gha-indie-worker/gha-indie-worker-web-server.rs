#![forbid(unsafe_code)]

use crate::config::WebConfig;
use crate::pages;

pub fn run(config: &WebConfig) {
    println!("web bind {}", config.bind);
    println!("{}", pages::home::markup());
}

