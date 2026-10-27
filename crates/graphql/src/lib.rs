mod account;
mod audit;
mod config;
mod context;
mod crypto;
mod enums;
mod error;
mod mutation;
mod ports;
mod query;
mod scalars;
mod skill;
mod task;
mod tenant;
mod trigger;

use async_graphql::{EmptySubscription, Schema};
use sanchoris_core::db::Store;

pub use context::{AuthUser, ClientIp};
pub use crypto::{AesGcmEncryptor, CREDENTIAL_KEY_ENV, LOCAL_KEY_ID};
pub use mutation::MutationRoot;
pub use ports::{
    AuditExportPort, ChatDispatcher, CheckoutPort, Clock, ConfigLocation, ConfigProblem,
    ConfigRepositoryPort, ConfigSyncTrigger, Encrypted, Encryptor, GithubConnectUrl, GraphqlDeps,
    PlanChange, PortError, SlackConnectUrl, SystemClock, TenantCheckout, TriggerProposal,
};
pub use query::QueryRoot;

pub type SanchorisSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

pub fn build_schema(store: Store, deps: GraphqlDeps) -> SanchorisSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(context::State { store, deps })
        .finish()
}

pub fn schema_sdl() -> String {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .finish()
        .sdl()
}
