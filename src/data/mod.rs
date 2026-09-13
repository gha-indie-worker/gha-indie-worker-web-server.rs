#![forbid(unsafe_code)]

//! The data layer, split by avenue.
//!
//! * [`db`] — **reads**. Read-only SeaORM against `DATABASE_URL_CANONICAL`.
//!   Fastest path for list and detail pages, and the only one that survives an
//!   api-server restart. Never writes, never runs DDL (fleet contract).
//! * [`api_client`] — **writes**, and reads the database cannot answer.
//!   Stateless HTTP to the api-server with the actor's bearer forwarded, so
//!   authorization is decided once, in one place.
//! * [`models`] — the view models both produce.
//!
//! [`Repo`] is the policy: read from the database when a pool exists, otherwise
//! from the api-server; write through the api-server, always. A page never picks
//! an avenue itself.

pub mod api_client;
#[cfg(feature = "db")]
pub mod db;
pub mod models;
pub mod rows;

use std::sync::Arc;

use crate::error::WebError;

pub use api_client::ApiClient;
pub use models::{
    AuditEntry, InviteOutcome, OrgMember, OrgSummary, PlanSummary, RunDetail, RunSummary, SeatUsage, TokenSummary,
    WorkerSummary,
};

/// Which avenue answered a read. Surfaced in `data-source` attributes and logs
/// so a slow page can be traced to the avenue that served it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadSource {
    Database,
    ApiServer,
}

impl ReadSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::ApiServer => "api-server",
        }
    }
}

/// The repository the handlers see.
#[derive(Clone)]
pub struct Repo {
    api: Arc<ApiClient>,
    #[cfg(feature = "db")]
    database: Option<Arc<db::ReadPool>>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Repo");
        debug.field("api", &self.api.base());
        #[cfg(feature = "db")]
        debug.field("database", &self.database.is_some());
        debug.finish()
    }
}

impl Repo {
    #[cfg(feature = "db")]
    #[must_use]
    pub fn new(api: Arc<ApiClient>, database: Option<Arc<db::ReadPool>>) -> Self {
        Self { api, database }
    }

    #[cfg(not(feature = "db"))]
    #[must_use]
    pub fn new(api: Arc<ApiClient>) -> Self {
        Self { api }
    }

    #[must_use]
    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    /// Which avenue a read will take right now.
    #[must_use]
    pub fn read_source(&self) -> ReadSource {
        #[cfg(feature = "db")]
        if self.database.is_some() {
            return ReadSource::Database;
        }
        ReadSource::ApiServer
    }

    /// Runs, newest first.
    pub async fn runs(
        &self,
        bearer: Option<&str>,
        org_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<RunSummary>, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.runs(org_id, limit).await;
        }
        let _ = org_id;
        self.api.runs(bearer, limit).await
    }

    /// One run with its jobs.
    pub async fn run(&self, bearer: Option<&str>, run_id: &str) -> Result<RunDetail, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.run(run_id).await;
        }
        self.api.run(bearer, run_id).await
    }

    /// The tail of a run's log. Always the api-server: logs are streamed state,
    /// not a canonical table this server may read.
    pub async fn run_log(
        &self,
        bearer: Option<&str>,
        run_id: &str,
        since: Option<u64>,
    ) -> Result<Vec<String>, WebError> {
        self.api.run_log(bearer, run_id, since).await
    }

    pub async fn workers(&self, bearer: Option<&str>, org_id: Option<&str>) -> Result<Vec<WorkerSummary>, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.workers(org_id).await;
        }
        let _ = org_id;
        self.api.workers(bearer).await
    }

    pub async fn plans(&self, bearer: Option<&str>) -> Result<Vec<PlanSummary>, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.plans().await;
        }
        self.api.plans(bearer).await
    }

    pub async fn members(&self, bearer: Option<&str>, org_id: &str) -> Result<Vec<OrgMember>, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.members(org_id).await;
        }
        self.api.members(bearer, org_id).await
    }

    pub async fn seats(&self, bearer: Option<&str>, org_id: &str) -> Result<SeatUsage, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.seats(org_id).await;
        }
        self.api.seats(bearer, org_id).await
    }

    pub async fn audit(&self, bearer: Option<&str>, org_id: &str, limit: u64) -> Result<Vec<AuditEntry>, WebError> {
        #[cfg(feature = "db")]
        if let Some(pool) = &self.database {
            return pool.audit(org_id, limit).await;
        }
        self.api.audit(bearer, org_id, limit).await
    }

    pub async fn tokens(&self, bearer: Option<&str>) -> Result<Vec<TokenSummary>, WebError> {
        // Token metadata is never read from the database: the api-server owns
        // redaction, and a token page must not be able to select a secret column.
        self.api.tokens(bearer).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repo {
        let api = Arc::new(ApiClient::for_tests());
        #[cfg(feature = "db")]
        {
            Repo::new(api, None)
        }
        #[cfg(not(feature = "db"))]
        {
            Repo::new(api)
        }
    }

    #[test]
    fn without_a_pool_every_read_goes_to_the_api_server() {
        assert_eq!(repo().read_source(), ReadSource::ApiServer);
        assert_eq!(ReadSource::Database.as_str(), "database");
    }

    #[test]
    fn the_debug_view_never_prints_a_connection_string() {
        let text = format!("{:?}", repo());
        assert!(!text.contains("postgres://"));
        assert!(!text.contains("password"));
    }
}
