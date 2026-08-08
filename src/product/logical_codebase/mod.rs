pub mod feature;
pub mod migration;
pub mod registry;
pub mod store;
pub mod types;

pub use feature::LogicalCodebaseFeature;
pub use migration::{
    IdentityMigrationExecutor, IdentityMigrationJournal, IdentityMigrationJournalStore,
    IdentityMigrationPhase, IdentityMigrationVerifier, MigrationFaultInjector,
    RepositoryIdentityMapping,
};
pub use registry::{
    IdentityRegistry, IdentityRegistryEntry, IdentityRegistryState, IdentityRegistryStore,
};
pub use store::{LogicalCodebaseLayout, LogicalCodebaseManifest, LogicalCodebaseStore};
pub use types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
