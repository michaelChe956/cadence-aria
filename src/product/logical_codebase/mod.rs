pub mod aggregate_index;
pub mod aggregate_initialization;
pub mod aggregate_initialization_coordinator;
pub mod aggregate_initialization_store;
pub mod feature;
pub mod issue_selection;
pub mod migration;
pub mod planning_context;
pub mod planning_context_resolver;
pub mod planning_context_set;
pub mod policy;
pub mod provider_gateway;
pub mod reference_scanner;
pub mod registration;
pub mod registry;
pub mod repository_routing;
pub mod store;
pub mod types;

pub use aggregate_initialization::{
    AGGREGATE_INITIALIZATION_LAYOUT_VERSION, AGGREGATE_INITIALIZATION_OPERATION_KIND,
    AggregateCancellationRecord, AggregateInitializationErrorRecord,
    AggregateInitializationIdempotencyIdentity, AggregateInitializationOperation,
    AggregateInitializationOperationInput, AggregateInitializationOperationStatus,
    AggregateInitializationProfile, AggregateInitializationStepKind,
    AggregateInitializationStepRecord, AggregateInitializationStepStatus, RepositoryTypeEvidence,
};
pub use aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError,
    AggregatePreflightMemberProjection, AggregatePreflightService, AggregatePreflightSnapshot,
    AggregateProviderTurnDriver, AggregateSkillsPreparation,
    DeterministicAggregatePreflightService, DeterministicRepositoryTypeDetector,
    MachineSkillsPreparation, RepositoryTypeDetector, profile_preflight_commands,
    resolve_aggregate_profile,
};
pub use aggregate_initialization_store::AggregateInitializationOperationStore;
pub use feature::LogicalCodebaseFeature;
pub use issue_selection::{
    EffectiveMemberResolution, InvalidationRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, SelectionPolicy,
};
pub use migration::{
    IdentityMigrationExecutor, IdentityMigrationJournal, IdentityMigrationJournalStore,
    IdentityMigrationPhase, IdentityMigrationVerifier, MigrationFaultInjector,
    RepositoryIdentityMapping,
};
pub use planning_context::{
    MemberCheckoutFingerprint, PlanningContextSnapshot, PlanningContextSnapshotStore,
};
pub use planning_context_resolver::{
    BestEffortReadonlyStatus, PlanningContextResolver, ResolvedPlanningContext, ResumeDecision,
};
pub use planning_context_set::{
    InventoryInjection, InventoryInjectionBudget, PlanningContextSetResolver,
    RepositoryContextResolution, RepositoryContextSet, render_compact_inventory,
};
pub use policy::{
    AggregatePolicyArtifact, AggregatePolicyArtifactStore, PolicyTarget, ProviderDialect,
    SessionPolicyAction, SessionPolicyEnvelope,
};
pub use provider_gateway::{
    GatewayRunAudit, GatewayRunAuditEntry, GatewayRunStack, LogicalCodebaseProviderGateway,
    PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderGatewayError,
    ProviderRef, ProviderRefType, SessionLaunchRequest, SessionResumeFingerprint,
    ValidatedSessionLaunchPolicy,
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
pub use repository_routing::{RepositoryRouting, RepositoryRoutingErrorCode};
pub use store::{LogicalCodebaseLayout, LogicalCodebaseManifest, LogicalCodebaseStore};
pub use types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
