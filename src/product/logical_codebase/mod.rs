pub mod aggregate_index;
pub mod feature;
pub mod migration;
pub mod policy;
pub mod reference_scanner;
pub mod registration;
pub mod registry;
pub mod store;
pub mod types;

pub use feature::LogicalCodebaseFeature;
pub use migration::{
    IdentityMigrationExecutor, IdentityMigrationJournal, IdentityMigrationJournalStore,
    IdentityMigrationPhase, IdentityMigrationVerifier, MigrationFaultInjector,
    RepositoryIdentityMapping,
};
pub use policy::{
    AggregatePolicyArtifact, AggregatePolicyArtifactStore, PolicyTarget, ProviderDialect,
    SessionPolicyAction, SessionPolicyEnvelope,
};
pub use reference_scanner::{
    RepositoryReference, RepositoryReferenceReport, RepositoryReferenceScanner,
};
pub use registration::{
    AggregateRootPreflight, AggregateRootPreflightError, AttachOnlyRegistrationInput,
    CanonicalAggregateRoot, DetectedRepositoryProfile, LogicalCodebaseRegistrationCoordinator,
    RegistrationCandidate, RegistrationCandidateState, RegistrationPreflightInput,
    RegistrationPreflightResult, RepositoryProfileDetector,
};
pub use registry::{
    IdentityRegistry, IdentityRegistryEntry, IdentityRegistryState, IdentityRegistryStore,
};
pub use store::{LogicalCodebaseLayout, LogicalCodebaseManifest, LogicalCodebaseStore};
pub use types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
