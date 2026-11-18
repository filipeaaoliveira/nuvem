//! The behavioural core: what the application can be asked to do, and what
//! each request does to its state.
//!
//! Every request is a variant of one [`Action`] enum, with a nested enum per
//! domain — never a stringly-typed command name. Reducers under `reducer/`
//! apply those actions to the state owned under `state/`, and are pure where
//! practical: when a transition needs the network, a subprocess, or the disk,
//! the reducer returns an [`Effect`] and the runtime dispatches it, handing
//! the outcome back as an `EffectResult` action.
//!
//! So this layer performs no I/O and draws nothing. `projection/` turns state
//! into the view models [`crate::ui`] renders, which keeps layout decisions
//! out of the reducers and state reads out of the widgets.

pub mod action;
pub mod effect;
pub mod projection;
pub mod reducer;
pub mod service_capabilities;
pub mod service_registry;
pub mod state;

pub use action::{
    Action, CommandAction, ConfigurationsAction, FinOpsAction, JumpsAction, LoggingAction,
    NavAction, NavigatorAction, ResourcesAction, SystemAction, TunnelsAction, WorkspaceAction,
};
pub use effect::{AppError, Effect, EffectResult, GcloudFailure, TunnelLaunchPlan};
pub use state::navigator::{NavigatorFocus, ServiceNavigatorState};
pub use state::resources::{CloudSqlInstanceAction, ComputeInstanceAction, ComputeInstanceField};
pub use state::root::{AppState, RootState, Workspace};
