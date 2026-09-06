#![forbid(unsafe_code)]
//! Read models and the small amount of state the web tier owns.
//!
//! The web tier is not the system of record. Runs, runners and memberships live in the canonical
//! Neon project and are written by the API server; this tier reads projections of them
//! (`transport::db` for analyst-shaped reads, `transport::http` for everything else).
//!
//! What is here today is an **in-memory fixture store** behind exactly the method signatures the
//! real projections will have, so the pages, the forms and the tenancy rules are all exercised end
//! to end without a database. Every mutating method already goes through
//! `lib_core::runtime::tenancy`, so replacing the storage does not move a single authorization
//! decision: the seat ledger is recomputed from rows on every read rather than kept as a counter,
//! for the same reason lib-core made [`Seats`] a value type.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use gha_indie_worker_lib_core::runtime::tenancy::{
    self, Invitation, InvitationState, JoinPolicy, OnboardingError, Role, Seats,
};

use crate::present::RunStatus;

/// Direct read-only ORM access. Kept from the original stub because `transport::db` names it.
#[derive(Clone, Debug, Default)]
pub struct ReadOnlyProjection {
    pub rows: usize,
}

/// One CI run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub id: String,
    pub repository: String,
    pub workflow: String,
    pub branch: String,
    pub commit: String,
    pub message: String,
    pub actor: String,
    pub status: RunStatus,
    /// Seconds since the epoch.
    pub started_at: i64,
    /// Wall-clock seconds, or `None` while it is still going.
    pub duration_seconds: Option<i64>,
    pub runner: Option<String>,
}

impl Run {
    /// The session stream carrying this run's log tail. The same name the API server publishes on.
    #[must_use]
    pub fn log_stream(&self) -> String {
        format!("run:{}:logs", self.id)
    }

    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self.status, RunStatus::Running | RunStatus::Queued)
    }
}

/// One self-hosted runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Runner {
    pub id: String,
    pub name: String,
    pub operating_system: String,
    pub architecture: String,
    pub labels: Vec<String>,
    pub online: bool,
    pub last_seen: i64,
    pub current_run: Option<String>,
}

/// One member of an organization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    /// The identity-provider subject. The stable key; the email is not.
    pub subject: String,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub joined_at: i64,
}

/// A domain an organization claims, and whether it has proved it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainClaim {
    pub domain: String,
    pub verified: bool,
    /// The DNS record the org must publish. Shown in full so the person can copy it.
    pub txt_name: String,
    pub txt_value: String,
}

/// Everything the `org.` surface renders.
#[derive(Clone, Debug)]
pub struct Organization {
    pub slug: String,
    pub name: String,
    /// Seats bought. Occupancy is never stored: see [`Organization::seats`].
    pub purchased_seats: u32,
    pub policy: JoinPolicy,
    pub members: Vec<Member>,
    pub invitations: Vec<Invitation>,
    pub domains: Vec<DomainClaim>,
    /// SSO is a placeholder in this release; the flag exists so the page can say so honestly.
    pub sso_configured: bool,
}

impl Organization {
    /// The seat ledger, computed from the rows every time.
    #[must_use]
    pub fn seats(&self, now: i64) -> Seats {
        Seats {
            purchased: self.purchased_seats,
            occupied: u32::try_from(self.members.len()).unwrap_or(u32::MAX),
            reserved: u32::try_from(
                self.invitations
                    .iter()
                    .filter(|invite| invite.is_open(now))
                    .count(),
            )
            .unwrap_or(u32::MAX),
        }
    }

    #[must_use]
    pub fn owner_count(&self) -> u32 {
        u32::try_from(
            self.members
                .iter()
                .filter(|member| member.role == Role::Owner)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Open invitations, newest first — the "pending" list on the invitations page.
    #[must_use]
    pub fn pending(&self, now: i64) -> Vec<&Invitation> {
        self.invitations
            .iter()
            .filter(|invite| invite.is_open(now))
            .collect()
    }
}

#[derive(Debug)]
struct Data {
    runs: Vec<Run>,
    logs: HashMap<String, Vec<String>>,
    runners: Vec<Runner>,
    organization: Organization,
}

/// The web tier's view of the world.
#[derive(Clone, Debug)]
pub struct Store {
    inner: Arc<RwLock<Data>>,
}

/// A fixed instant the fixtures are written against, so a demo never renders "in 3 hours".
const EPOCH: i64 = 1_788_600_000;

impl Store {
    /// A store seeded with a plausible small organization.
    #[must_use]
    pub fn with_fixtures() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Data::fixtures())),
        }
    }

    fn read<T>(&self, f: impl FnOnce(&Data) -> T) -> T
    where
        T: Default,
    {
        self.inner.read().map(|guard| f(&guard)).unwrap_or_default()
    }

    #[must_use]
    pub fn runs(&self) -> Vec<Run> {
        self.read(|data| data.runs.clone())
    }

    #[must_use]
    pub fn run(&self, id: &str) -> Option<Run> {
        self.read(|data| data.runs.iter().find(|run| run.id == id).cloned())
    }

    /// The stored log for a run. The live tail continues from the end of this.
    #[must_use]
    pub fn log_lines(&self, run_id: &str) -> Vec<String> {
        self.read(|data| data.logs.get(run_id).cloned().unwrap_or_default())
    }

    #[must_use]
    pub fn runners(&self) -> Vec<Runner> {
        self.read(|data| data.runners.clone())
    }

    #[must_use]
    pub fn organization(&self) -> Organization {
        self.inner
            .read()
            .map(|guard| guard.organization.clone())
            .unwrap_or_else(|_| Data::fixtures().organization)
    }

    fn write<T>(&self, f: impl FnOnce(&mut Organization) -> T) -> Result<T, OnboardingError> {
        // A poisoned lock means another thread panicked while holding it. Refusing the write is
        // the safe answer; `InvitationNotOpen` is the closest honest thing to say about a
        // membership change that did not happen.
        let mut guard = self
            .inner
            .write()
            .map_err(|_| OnboardingError::InvitationNotOpen)?;
        Ok(f(&mut guard.organization))
    }

    /// Invite someone. The authorization is `tenancy::authorize_invitation`; this method only
    /// records the row it returned.
    ///
    /// # Errors
    /// Any [`OnboardingError`] the tenancy rules produce.
    pub fn invite(
        &self,
        actor: Role,
        email: &str,
        granted: Role,
        now: i64,
    ) -> Result<Invitation, OnboardingError> {
        let organization = self.organization();
        let seats = organization.seats(now);
        let normalized =
            tenancy::authorize_invitation(actor, granted, email, seats, &organization.policy)?;
        // Re-inviting an address that already has an open invitation is a resend, not a second
        // seat: two open invitations for one person would reserve two seats for one hire.
        if organization
            .invitations
            .iter()
            .any(|invite| invite.email == normalized && invite.is_open(now))
        {
            return Err(OnboardingError::InvitationNotOpen);
        }
        if organization
            .members
            .iter()
            .any(|member| member.email == normalized)
        {
            return Err(OnboardingError::InvitationNotOpen);
        }
        let invitation = Invitation {
            email: normalized,
            role: granted,
            state: InvitationState::Open,
            expires_at: now + 7 * 24 * 60 * 60,
        };
        let stored = invitation.clone();
        self.write(move |organization| organization.invitations.insert(0, stored))?;
        Ok(invitation)
    }

    /// Withdraw an open invitation, freeing the seat it reserved.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`] when the actor may not administer invitations,
    /// [`OnboardingError::InvitationNotOpen`] when there is nothing open at that address.
    pub fn revoke_invitation(
        &self,
        actor: Role,
        email: &str,
        now: i64,
    ) -> Result<(), OnboardingError> {
        if !actor.at_least(Role::Admin) {
            return Err(OnboardingError::RoleEscalation {
                actor,
                granted: Role::Viewer,
            });
        }
        let (normalized, _) = tenancy::parse_email(email)?;
        self.write(|organization| {
            let found = organization
                .invitations
                .iter_mut()
                .find(|invite| invite.email == normalized && invite.is_open(now));
            match found {
                Some(invitation) => {
                    invitation.state = InvitationState::Revoked;
                    Ok(())
                }
                None => Err(OnboardingError::InvitationNotOpen),
            }
        })?
    }

    /// Push an open invitation's expiry out and (in the real system) send the mail again.
    ///
    /// # Errors
    /// As [`Store::revoke_invitation`].
    pub fn resend_invitation(
        &self,
        actor: Role,
        email: &str,
        now: i64,
    ) -> Result<Invitation, OnboardingError> {
        if !actor.at_least(Role::Admin) {
            return Err(OnboardingError::RoleEscalation {
                actor,
                granted: Role::Viewer,
            });
        }
        let (normalized, _) = tenancy::parse_email(email)?;
        self.write(|organization| {
            let found = organization
                .invitations
                .iter_mut()
                .find(|invite| invite.email == normalized && invite.is_open(now));
            match found {
                Some(invitation) => {
                    invitation.expires_at = now + 7 * 24 * 60 * 60;
                    Ok(invitation.clone())
                }
                None => Err(OnboardingError::InvitationNotOpen),
            }
        })?
    }

    /// Change a member's role.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`], [`OnboardingError::LastOwner`].
    pub fn change_role(
        &self,
        actor: Role,
        subject: &str,
        next: Role,
    ) -> Result<Member, OnboardingError> {
        let organization = self.organization();
        let member = organization
            .members
            .iter()
            .find(|member| member.subject == subject)
            .ok_or(OnboardingError::InvitationNotOpen)?;
        tenancy::authorize_role_change(actor, member.role, next, organization.owner_count())?;
        let subject = subject.to_owned();
        self.write(move |organization| {
            let member = organization
                .members
                .iter_mut()
                .find(|member| member.subject == subject)
                .ok_or(OnboardingError::InvitationNotOpen)?;
            member.role = next;
            Ok(member.clone())
        })?
    }

    /// Remove a member, freeing their seat.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`], [`OnboardingError::LastOwner`].
    pub fn remove_member(&self, actor: Role, subject: &str) -> Result<(), OnboardingError> {
        let organization = self.organization();
        let member = organization
            .members
            .iter()
            .find(|member| member.subject == subject)
            .ok_or(OnboardingError::InvitationNotOpen)?;
        tenancy::authorize_removal(actor, member.role, organization.owner_count())?;
        let subject = subject.to_owned();
        self.write(move |organization| {
            organization
                .members
                .retain(|member| member.subject != subject);
        })
    }

    /// Claim a domain. Verification is a separate step, and until it happens the claim grants
    /// nothing at all.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`], [`OnboardingError::InvalidEmail`] for a domain that is
    /// not shaped like one.
    pub fn claim_domain(&self, actor: Role, domain: &str) -> Result<DomainClaim, OnboardingError> {
        if !actor.at_least(Role::Admin) {
            return Err(OnboardingError::RoleEscalation {
                actor,
                granted: Role::Admin,
            });
        }
        // Borrow the address validator: `admin@<domain>` is valid exactly when `<domain>` is.
        let (_, domain) = tenancy::parse_email(&format!("admin@{}", domain.trim()))?;
        let claim = DomainClaim {
            txt_name: format!("_indiebuild-verify.{domain}"),
            txt_value: format!(
                "indiebuild-site-verification={}",
                crate::auth::random_token()
            ),
            domain,
            verified: false,
        };
        let stored = claim.clone();
        self.write(move |organization| {
            if let Some(existing) = organization
                .domains
                .iter_mut()
                .find(|claim| claim.domain == stored.domain)
            {
                *existing = stored;
            } else {
                organization.domains.push(stored);
            }
        })?;
        Ok(claim)
    }

    /// Mark a claim verified. In the real system this is the result of a DNS lookup; here it is
    /// the button, and the page says so.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`], [`OnboardingError::DomainNotAllowed`] when nothing is
    /// claimed at that domain.
    pub fn verify_domain(&self, actor: Role, domain: &str) -> Result<(), OnboardingError> {
        if !actor.at_least(Role::Admin) {
            return Err(OnboardingError::RoleEscalation {
                actor,
                granted: Role::Admin,
            });
        }
        let domain = domain.trim().to_ascii_lowercase();
        self.write(|organization| {
            let Some(claim) = organization
                .domains
                .iter_mut()
                .find(|claim| claim.domain == domain)
            else {
                return Err(OnboardingError::DomainNotAllowed {
                    domain: domain.clone(),
                });
            };
            claim.verified = true;
            if !organization
                .policy
                .verified_domains
                .iter()
                .any(|existing| *existing == domain)
            {
                organization.policy.verified_domains.push(domain.clone());
            }
            Ok(())
        })?
    }

    /// Turn joining-by-verified-domain on or off.
    ///
    /// # Errors
    /// [`OnboardingError::RoleEscalation`].
    pub fn set_domain_join(&self, actor: Role, enabled: bool) -> Result<(), OnboardingError> {
        if !actor.at_least(Role::Admin) {
            return Err(OnboardingError::RoleEscalation {
                actor,
                granted: Role::Admin,
            });
        }
        self.write(|organization| organization.policy.domain_join_enabled = enabled)
    }
}

impl Data {
    fn fixtures() -> Self {
        let runs = vec![
            Run {
                id: "01HZY7Q0J8KDPM4V2XN6ABCD".into(),
                repository: "acme/checkout".into(),
                workflow: "CI".into(),
                branch: "main".into(),
                commit: "9f2c1ab".into(),
                message: "Cache the resolver between jobs".into(),
                actor: "alex".into(),
                status: RunStatus::Running,
                started_at: EPOCH - 92,
                duration_seconds: None,
                runner: Some("hetzner-arm-01".into()),
            },
            Run {
                id: "01HZY7APQ3RS8T1U5WV9EFGH".into(),
                repository: "acme/checkout".into(),
                workflow: "CI".into(),
                branch: "renovate/axum".into(),
                commit: "3d7e004".into(),
                message: "Bump axum to 0.8.9".into(),
                actor: "renovate[bot]".into(),
                status: RunStatus::Failed,
                started_at: EPOCH - 3_600,
                duration_seconds: Some(214),
                runner: Some("hetzner-arm-02".into()),
            },
            Run {
                id: "01HZY6ZK2M7NB3C4D5E6IJKL".into(),
                repository: "acme/billing".into(),
                workflow: "Release".into(),
                branch: "main".into(),
                commit: "c81b55e".into(),
                message: "Release 2.14.0".into(),
                actor: "priya".into(),
                status: RunStatus::Succeeded,
                started_at: EPOCH - 7_800,
                duration_seconds: Some(486),
                runner: Some("hetzner-x86-01".into()),
            },
            Run {
                id: "01HZY6TVA9WX0Y1Z2A3B4MNO".into(),
                repository: "acme/checkout".into(),
                workflow: "Nightly".into(),
                branch: "main".into(),
                commit: "51aa9c2".into(),
                message: "Nightly integration suite".into(),
                actor: "schedule".into(),
                status: RunStatus::Cancelled,
                started_at: EPOCH - 40_000,
                duration_seconds: Some(31),
                runner: None,
            },
            Run {
                id: "01HZY8B1C2D3E4F5G6H7PQRS".into(),
                repository: "acme/billing".into(),
                workflow: "CI".into(),
                branch: "feat/ledger".into(),
                commit: "77c0d19".into(),
                message: "Split the ledger writer".into(),
                actor: "sam".into(),
                status: RunStatus::Queued,
                started_at: EPOCH - 12,
                duration_seconds: None,
                runner: None,
            },
        ];

        let mut logs = HashMap::new();
        logs.insert(
            runs[0].id.clone(),
            [
                "Runner hetzner-arm-01 (linux/arm64) accepted job build",
                "Restoring cache: cargo-registry-arm64-9f2c1ab",
                "Cache restored in 1.8s (412 MiB)",
                "+ cargo fmt --all -- --check",
                "+ cargo clippy --locked --all-targets -- -D warnings",
                "    Checking gha-indie-worker-web-server v0.1.0",
                "    Finished dev profile in 41.2s",
                "+ cargo test --locked --all-targets",
                "running 61 tests",
                "test policy::tests::no_policy_ever_contains_an_unsafe_source ... ok",
                "test csrf::tests::a_planted_cookie_that_matches_the_field_still_fails_once_signed_in ... ok",
            ]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
        );
        logs.insert(
            runs[1].id.clone(),
            [
                "Runner hetzner-arm-02 (linux/arm64) accepted job build",
                "+ cargo clippy --locked --all-targets -- -D warnings",
                "error: this expression creates a reference which is immediately dereferenced",
                "  --> src/routes/mod.rs:41:18",
                "error: could not compile `gha-indie-worker-web-server` (lib) due to 1 previous error",
                "Job failed after 3m 34s",
            ]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
        );

        let runners = vec![
            Runner {
                id: "rnr_01".into(),
                name: "hetzner-arm-01".into(),
                operating_system: "Ubuntu 24.04".into(),
                architecture: "arm64".into(),
                labels: vec!["self-hosted".into(), "linux".into(), "arm64".into()],
                online: true,
                last_seen: EPOCH - 3,
                current_run: Some(runs[0].id.clone()),
            },
            Runner {
                id: "rnr_02".into(),
                name: "hetzner-arm-02".into(),
                operating_system: "Ubuntu 24.04".into(),
                architecture: "arm64".into(),
                labels: vec!["self-hosted".into(), "linux".into(), "arm64".into()],
                online: true,
                last_seen: EPOCH - 11,
                current_run: None,
            },
            Runner {
                id: "rnr_03".into(),
                name: "hetzner-x86-01".into(),
                operating_system: "Ubuntu 24.04".into(),
                architecture: "x86-64".into(),
                labels: vec!["self-hosted".into(), "linux".into(), "x64".into()],
                online: false,
                last_seen: EPOCH - 5_400,
                current_run: None,
            },
        ];

        let organization = Organization {
            slug: "acme".into(),
            name: "Acme Corp".into(),
            purchased_seats: 5,
            policy: JoinPolicy {
                verified_domains: vec!["acme.test".into()],
                domain_join_enabled: false,
                restrict_invitations_to_verified_domains: false,
                default_role: Role::Viewer,
            },
            members: vec![
                Member {
                    subject: "sub_alex".into(),
                    email: "alex@acme.test".into(),
                    name: "Alex Mills".into(),
                    role: Role::Owner,
                    joined_at: EPOCH - 400 * 86_400,
                },
                Member {
                    subject: "sub_priya".into(),
                    email: "priya@acme.test".into(),
                    name: "Priya Raman".into(),
                    role: Role::Admin,
                    joined_at: EPOCH - 210 * 86_400,
                },
                Member {
                    subject: "sub_sam".into(),
                    email: "sam@acme.test".into(),
                    name: "Sam Okafor".into(),
                    role: Role::Member,
                    joined_at: EPOCH - 45 * 86_400,
                },
            ],
            invitations: vec![Invitation {
                email: "jo@acme.test".into(),
                role: Role::Member,
                state: InvitationState::Open,
                expires_at: EPOCH + 5 * 86_400,
            }],
            domains: vec![DomainClaim {
                domain: "acme.test".into(),
                verified: true,
                txt_name: "_indiebuild-verify.acme.test".into(),
                txt_value: "indiebuild-site-verification=fixture".into(),
            }],
            sso_configured: false,
        };

        Self {
            runs,
            logs,
            runners,
            organization,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = EPOCH;

    #[test]
    fn the_seat_ledger_is_computed_from_rows_and_never_stored() {
        let store = Store::with_fixtures();
        let seats = store.organization().seats(NOW);
        assert_eq!(
            seats,
            Seats {
                purchased: 5,
                occupied: 3,
                reserved: 1
            }
        );
        assert_eq!(seats.available(), 1);

        store
            .invite(Role::Owner, "kim@acme.test", Role::Member, NOW)
            .unwrap();
        assert_eq!(store.organization().seats(NOW).available(), 0);

        // Revoking gives the seat straight back, because "reserved" is a query over open rows.
        store
            .revoke_invitation(Role::Owner, "kim@acme.test", NOW)
            .unwrap();
        assert_eq!(store.organization().seats(NOW).available(), 1);
    }

    #[test]
    fn inviting_past_the_last_seat_is_refused_with_the_ledger_in_the_error() {
        let store = Store::with_fixtures();
        store
            .invite(Role::Owner, "kim@acme.test", Role::Member, NOW)
            .unwrap();
        let error = store
            .invite(Role::Owner, "lee@acme.test", Role::Member, NOW)
            .unwrap_err();
        assert_eq!(
            error,
            OnboardingError::NoSeatsAvailable {
                purchased: 5,
                occupied: 3,
                reserved: 2
            }
        );
    }

    #[test]
    fn the_same_address_cannot_hold_two_open_invitations_or_a_seat_it_already_has() {
        let store = Store::with_fixtures();
        assert_eq!(
            store
                .invite(Role::Owner, "JO@acme.test", Role::Member, NOW)
                .unwrap_err(),
            OnboardingError::InvitationNotOpen
        );
        assert_eq!(
            store
                .invite(Role::Owner, "alex@acme.test", Role::Member, NOW)
                .unwrap_err(),
            OnboardingError::InvitationNotOpen
        );
    }

    #[test]
    fn only_an_admin_may_touch_invitations() {
        let store = Store::with_fixtures();
        for actor in [Role::Viewer, Role::Member] {
            assert!(matches!(
                store.invite(actor, "kim@acme.test", Role::Viewer, NOW),
                Err(OnboardingError::RoleEscalation { .. })
            ));
            assert!(matches!(
                store.revoke_invitation(actor, "jo@acme.test", NOW),
                Err(OnboardingError::RoleEscalation { .. })
            ));
            assert!(matches!(
                store.resend_invitation(actor, "jo@acme.test", NOW),
                Err(OnboardingError::RoleEscalation { .. })
            ));
        }
    }

    #[test]
    fn resending_extends_the_invitation_rather_than_making_a_second_one() {
        let store = Store::with_fixtures();
        let before = store.organization().invitations.len();
        let extended = store
            .resend_invitation(Role::Admin, "jo@acme.test", NOW)
            .unwrap();
        assert_eq!(extended.expires_at, NOW + 7 * 86_400);
        assert_eq!(store.organization().invitations.len(), before);
        assert_eq!(store.organization().seats(NOW).reserved, 1);
    }

    #[test]
    fn the_last_owner_cannot_be_demoted_or_removed() {
        let store = Store::with_fixtures();
        assert_eq!(
            store
                .change_role(Role::Owner, "sub_alex", Role::Admin)
                .unwrap_err(),
            OnboardingError::LastOwner
        );
        assert_eq!(
            store.remove_member(Role::Owner, "sub_alex").unwrap_err(),
            OnboardingError::LastOwner
        );
        // With a second owner, both become possible.
        store
            .change_role(Role::Owner, "sub_priya", Role::Owner)
            .unwrap();
        assert_eq!(store.organization().owner_count(), 2);
        assert!(store
            .change_role(Role::Owner, "sub_alex", Role::Admin)
            .is_ok());
    }

    #[test]
    fn an_admin_cannot_promote_anyone_to_owner() {
        let store = Store::with_fixtures();
        assert!(matches!(
            store.change_role(Role::Admin, "sub_sam", Role::Owner),
            Err(OnboardingError::RoleEscalation { .. })
        ));
        assert!(matches!(
            store.remove_member(Role::Admin, "sub_alex"),
            Err(OnboardingError::RoleEscalation { .. })
        ));
        assert!(store
            .change_role(Role::Admin, "sub_sam", Role::Admin)
            .is_ok());
    }

    #[test]
    fn a_domain_grants_nothing_until_it_is_verified() {
        let store = Store::with_fixtures();
        let claim = store.claim_domain(Role::Admin, " Acme-Labs.Test ").unwrap();
        assert_eq!(claim.domain, "acme-labs.test");
        assert!(!claim.verified);
        assert_eq!(claim.txt_name, "_indiebuild-verify.acme-labs.test");
        assert!(!store
            .organization()
            .policy
            .verified_domains
            .contains(&claim.domain));

        store.verify_domain(Role::Admin, "acme-labs.test").unwrap();
        assert!(store
            .organization()
            .policy
            .verified_domains
            .contains(&"acme-labs.test".to_owned()));
        // Claiming it again replaces the record rather than adding a duplicate.
        store.claim_domain(Role::Owner, "acme-labs.test").unwrap();
        assert_eq!(
            store
                .organization()
                .domains
                .iter()
                .filter(|c| c.domain == "acme-labs.test")
                .count(),
            1
        );
        assert!(matches!(
            store.verify_domain(Role::Admin, "never-claimed.test"),
            Err(OnboardingError::DomainNotAllowed { .. })
        ));
        assert_eq!(
            store.claim_domain(Role::Admin, "not a domain").unwrap_err(),
            OnboardingError::InvalidEmail
        );
    }

    #[test]
    fn a_restricted_organization_refuses_an_outside_address_by_name() {
        let store = Store::with_fixtures();
        store
            .write(|organization| {
                organization.policy.restrict_invitations_to_verified_domains = true
            })
            .unwrap();
        assert_eq!(
            store
                .invite(Role::Owner, "kim@contractor.example", Role::Member, NOW)
                .unwrap_err(),
            OnboardingError::DomainNotAllowed {
                domain: "contractor.example".into()
            }
        );
        assert!(store
            .invite(Role::Owner, "kim@acme.test", Role::Member, NOW)
            .is_ok());
    }

    #[test]
    fn runs_and_their_streams_line_up() {
        let store = Store::with_fixtures();
        let runs = store.runs();
        assert_eq!(runs.len(), 5);
        let live = runs.iter().find(|run| run.is_live()).unwrap();
        assert_eq!(live.log_stream(), format!("run:{}:logs", live.id));
        assert!(!store.log_lines(&runs[0].id).is_empty());
        assert!(store.log_lines("nope").is_empty());
        assert!(store.run(&runs[0].id).is_some());
        assert!(store.run("nope").is_none());
        assert_eq!(store.runners().len(), 3);
    }
}
