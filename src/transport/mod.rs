#![forbid(unsafe_code)]

pub mod db;
pub mod http;
pub mod nats;
pub mod tcp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Avenue {
    DirectReadOnlyDatabase,
    StatelessHttp,
    StatefulMtlsTcp,
    DurableNats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    AnalystProjection,
    ApiRead,
    StatefulStatus,
    AsyncStatus,
}

pub fn choose(operation: Operation) -> Avenue {
    match operation {
        Operation::AnalystProjection => Avenue::DirectReadOnlyDatabase,
        Operation::ApiRead => Avenue::StatelessHttp,
        Operation::StatefulStatus => Avenue::StatefulMtlsTcp,
        Operation::AsyncStatus => Avenue::DurableNats,
    }
}

