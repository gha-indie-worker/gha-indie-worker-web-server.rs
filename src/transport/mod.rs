#![forbid(unsafe_code)]
//! Four ways this tier reaches the rest of the system, and the rule for choosing between them.
//!
//! The web tier is a **renderer**. It owns sessions and nothing else; runs, runners and
//! memberships are the API server's, and the four avenues below are how it asks. Keeping the
//! choice in one total function rather than in each call site is what stops a page quietly opening
//! a database connection because that was easier than an HTTP call.
//!
//! | avenue | for | why not the others |
//! |---|---|---|
//! | direct read-only database | analyst-shaped projections over many rows | an HTTP round trip per row is not a query |
//! | stateless HTTP | ordinary reads and writes | the default; cacheable, traceable, boring |
//! | stateful mTLS TCP | long-lived status from a runner | an HTTP request per heartbeat is not a session |
//! | durable NATS | work that must survive this process | a request that must not be lost is not a request |

pub mod db;
pub mod http;
pub mod nats;
pub mod tcp;

/// Which avenue a call takes.
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
            Avenue::DirectReadOnlyDatabase => "direct-read-only-database",
            Avenue::StatelessHttp => "stateless-http",
            Avenue::StatefulMtlsTcp => "stateful-mtls-tcp",
            Avenue::DurableNats => "durable-nats",
        }
    }

    /// Whether a page render may block on this avenue. A page that waits on a durable queue is a
    /// page that hangs when the queue is slow, which is exactly when somebody is looking at it.
    #[must_use]
    pub const fn may_block_a_render(self) -> bool {
        matches!(self, Avenue::DirectReadOnlyDatabase | Avenue::StatelessHttp)
    }
}

/// What the caller is trying to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    AnalystProjection,
    ApiRead,
    StatefulStatus,
    AsyncStatus,
}

/// The avenue for an operation. Total, so a new operation cannot default into the database.
#[must_use]
pub const fn choose(operation: Operation) -> Avenue {
    match operation {
        Operation::AnalystProjection => Avenue::DirectReadOnlyDatabase,
        Operation::ApiRead => Avenue::StatelessHttp,
        Operation::StatefulStatus => Avenue::StatefulMtlsTcp,
        Operation::AsyncStatus => Avenue::DurableNats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_has_exactly_one_avenue_and_they_are_all_distinct() {
        let operations = [
            Operation::AnalystProjection,
            Operation::ApiRead,
            Operation::StatefulStatus,
            Operation::AsyncStatus,
        ];
        let mut seen: Vec<Avenue> = Vec::new();
        for operation in operations {
            let avenue = choose(operation);
            assert!(!seen.contains(&avenue), "{operation:?} reuses {avenue:?}");
            seen.push(avenue);
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn a_page_render_never_waits_on_a_queue_or_a_socket() {
        assert!(choose(Operation::ApiRead).may_block_a_render());
        assert!(choose(Operation::AnalystProjection).may_block_a_render());
        assert!(!choose(Operation::AsyncStatus).may_block_a_render());
        assert!(!choose(Operation::StatefulStatus).may_block_a_render());
    }
}
