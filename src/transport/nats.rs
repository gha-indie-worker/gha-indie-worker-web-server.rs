#![forbid(unsafe_code)]

#[derive(Clone, Debug)]
pub struct NatsTransport {
    pub subject: String,
}

