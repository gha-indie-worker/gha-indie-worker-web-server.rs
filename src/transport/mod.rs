#![forbid(unsafe_code)]

//! The four interaction avenues, and the rule for choosing between them.
//!
//! | avenue | module | used for |
//! |---|---|---|
//! | direct read-only SeaORM | [`db`] | list and detail pages |
//! | stateless HTTP | [`http`] | every write, and reads the database cannot answer |
//! | stateful TCP + WebSocket | [`tcp`], [`crate::ws`] | operator tooling, live log streaming |
//! | async NATS/JetStream | [`nats`] | fan-out events nobody is waiting on |
//!
//! [`choose`] is the whole policy, as an exhaustive match: adding an operation
//! without deciding its avenue does not compile.

pub mod db;
pub mod http;
#[cfg(feature = "nats-transport")]
pub mod nats;
#[cfg(feature = "tcp-transport")]
pub mod tcp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Avenue {
    DirectReadOnlyDatabase,
    StatelessHttp,
    StatefulMtlsTcp,
    DurableNats,
}

impl Avenue {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DirectReadOnlyDatabase => "database",
            Self::StatelessHttp => "http",
            Self::StatefulMtlsTcp => "tcp",
            Self::DurableNats => "nats",
        }
    }
}

/// Everything this server does, classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// A page listing rows a human will read.
    AnalystProjection,
    /// A read the api-server owns (logs, token metadata).
    ApiRead,
    /// Any state change at all.
    Mutation,
    /// A long-lived operator connection.
    StatefulStatus,
    /// Live log or chat frames for an open page.
    LiveStream,
    /// An event nobody is waiting on.
    AsyncStatus,
}

/// The avenue policy. Exhaustive on purpose.
#[must_use]
pub const fn choose(operation: Operation) -> Avenue {
    match operation {
        Operation::AnalystProjection => Avenue::DirectReadOnlyDatabase,
        // Writes never touch the database directly: the api-server owns the
        // domain rules, and this server holds a read-only credential.
        Operation::ApiRead | Operation::Mutation => Avenue::StatelessHttp,
        Operation::StatefulStatus => Avenue::StatefulMtlsTcp,
        // The relay is a WebSocket on the HTTP port, upgraded from the same
        // stateful avenue the operator TCP listener serves.
        Operation::LiveStream => Avenue::StatefulMtlsTcp,
        Operation::AsyncStatus => Avenue::DurableNats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_always_go_through_the_api_server() {
        assert_eq!(choose(Operation::Mutation), Avenue::StatelessHttp);
        assert_eq!(choose(Operation::ApiRead), Avenue::StatelessHttp);
    }

    #[test]
    fn page_reads_prefer_the_canonical_database() {
        assert_eq!(choose(Operation::AnalystProjection), Avenue::DirectReadOnlyDatabase);
    }

    #[test]
    fn streaming_shares_the_stateful_avenue() {
        assert_eq!(choose(Operation::LiveStream), Avenue::StatefulMtlsTcp);
        assert_eq!(choose(Operation::StatefulStatus), Avenue::StatefulMtlsTcp);
    }

    #[test]
    fn every_avenue_has_a_stable_name() {
        let names: Vec<&str> = [
            Avenue::DirectReadOnlyDatabase,
            Avenue::StatelessHttp,
            Avenue::StatefulMtlsTcp,
            Avenue::DurableNats,
        ]
        .iter()
        .map(|avenue| avenue.as_str())
        .collect();
        assert_eq!(names, vec!["database", "http", "tcp", "nats"]);
    }
}
