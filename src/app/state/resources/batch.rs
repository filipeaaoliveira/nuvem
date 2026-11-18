use super::*;

/// A batch target, already projected to everything the confirmation modal,
/// the progress panel, and the runtime need — so no batch type is generic
/// over the row type and the UI has one rendering path per service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRowRef {
    pub name: String,
    /// Zone for GCE, region for Cloud SQL. Part of target identity.
    pub location: String,
    /// Pre-rendered status text, e.g. "Running" or "Unknown (FOO)".
    pub status_label: String,
    /// The call that will run for this target, shown verbatim in the
    /// confirmation modal.
    ///
    /// One per target and never summarised: marks can be hidden by an active
    /// filter, so this list is the only thing between an operator and acting
    /// on rows they cannot see. That argument covers what each line says as
    /// much as how many there are.
    pub call: ResourceCall,
}

/// The lifecycle action a batch applies, tagged with the service it targets so
/// confirmation and workflow state stay service-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAction {
    Compute(ComputeInstanceAction),
    CloudSql(CloudSqlInstanceAction),
}

impl BatchAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Compute(action) => action.label(),
            Self::CloudSql(action) => action.label(),
        }
    }

    /// True for actions whose effect is destructive enough to warrant an
    /// explicit typed-count acknowledgement in a batch confirmation.
    pub fn is_destructive(self) -> bool {
        match self {
            Self::Compute(action) => action.is_destructive(),
            Self::CloudSql(action) => action.is_destructive(),
        }
    }

    pub fn service_kind(self) -> ResourceServiceKind {
        match self {
            Self::Compute(_) => ResourceServiceKind::ComputeInstances,
            Self::CloudSql(_) => ResourceServiceKind::CloudSql,
        }
    }

    /// How the batch surface names this action's service: "GCE" for Compute,
    /// "Cloud SQL" for CloudSql.
    ///
    /// Every user-visible batch string that names a service is built from this,
    /// so the wording has one source of truth per service. Spelling it out per
    /// panel is what let the progress panel keep calling every batch a GCE one
    /// after the confirmation modal learned about Cloud SQL.
    pub fn service_label(self) -> &'static str {
        match self {
            Self::Compute(_) => "GCE",
            Self::CloudSql(_) => "Cloud SQL",
        }
    }

    /// Header line of the confirmation modal.
    pub fn title(self) -> String {
        format!("Confirm {} batch action", self.service_label())
    }

    /// Border title of the confirmation modal's block, in the terse lowercase
    /// shape the other modal borders use. Names the service the batch targets
    /// so the frame around a Cloud SQL batch does not read as a GCE one.
    pub fn modal_border_title(self) -> String {
        format!("confirm {} batch", self.service_label())
    }

    /// Body shown in place of the confirmation modal when the terminal is too
    /// small to render it. Names the same service as
    /// [`BatchAction::modal_border_title`], so the fallback never disagrees
    /// with the border drawn around it.
    pub fn small_terminal_message(self) -> String {
        format!(
            "nuvem {} batch confirmation needs a larger terminal",
            self.service_label()
        )
    }

    /// The progress panel's twin of [`BatchAction::small_terminal_message`],
    /// shown when the terminal is too small for the in-flight batch's panel. It
    /// names the service whose batch is running for the same reason the
    /// confirmation fallback does.
    pub fn progress_small_terminal_message(self) -> String {
        format!(
            "nuvem {} batch progress needs a larger terminal",
            self.service_label()
        )
    }
}

/// Renders a [`ResourceStatus`] the way the batch modal reports it: exactly as
/// the table's STATUS cell paints it, unrecognised values included, so a skip
/// line in the modal can be matched back to the row it names. Lives next to the
/// batch types so the state layer can pre-render the label a [`BatchRowRef`]
/// carries.
pub fn resource_status_label(status: &ResourceStatus) -> String {
    nuvem_domain::gcp::text::sanitize_display_text(&status.label())
}

/// The Cloud SQL twin of [`resource_status_label`].
pub fn cloud_sql_status_label(status: &CloudSqlStatus) -> String {
    nuvem_domain::gcp::text::sanitize_display_text(&status.label())
}

/// A [`ResourceService`] whose rows can be targeted by a marked-row batch
/// action: it knows its own action type, how to tag it for the shared batch
/// state, and how to project a row into a [`BatchRowRef`].
pub trait BatchableService: ResourceService {
    type Action: Copy;
    fn wrap(action: Self::Action) -> BatchAction;
    fn is_eligible(row: &Self::Row, action: Self::Action) -> bool;
    fn row_ref(row: &Self::Row, context: &GcloudContext, action: Self::Action) -> BatchRowRef;

    /// Explains why an ineligible `row` was skipped. Defaults to reporting the
    /// row's rendered status label, which is the whole story for a service
    /// whose eligibility depends on status alone; a service that gates on more
    /// than status overrides this so the skip is never reported by a state
    /// that reads as eligible.
    fn skip_reason(_row: &Self::Row, row_ref: &BatchRowRef, _action: Self::Action) -> SkipReason {
        SkipReason::Ineligible(row_ref.status_label.clone())
    }
}

impl BatchableService for Compute {
    type Action = ComputeInstanceAction;

    fn wrap(action: Self::Action) -> BatchAction {
        BatchAction::Compute(action)
    }

    fn is_eligible(row: &Self::Row, action: Self::Action) -> bool {
        action.is_eligible(&row.status)
    }

    fn row_ref(row: &Self::Row, context: &GcloudContext, action: Self::Action) -> BatchRowRef {
        BatchRowRef {
            name: row.name.clone(),
            location: row.zone.clone(),
            status_label: resource_status_label(&row.status),
            call: compute_lifecycle_call(context, &row.name, &row.zone, action),
        }
    }
}

impl BatchableService for CloudSql {
    type Action = CloudSqlInstanceAction;

    fn wrap(action: Self::Action) -> BatchAction {
        BatchAction::CloudSql(action)
    }

    /// Cloud SQL eligibility needs the row's availability as well as its
    /// status: `Failover` only applies to a regional (HA) instance.
    fn is_eligible(row: &Self::Row, action: Self::Action) -> bool {
        action.is_eligible(&row.status, row.availability)
    }

    /// A row whose status would have passed and that only failed on
    /// availability is reported as such: `Failover` on a zonal instance is
    /// skipped for the missing standby, not for being `Runnable`.
    fn skip_reason(row: &Self::Row, row_ref: &BatchRowRef, action: Self::Action) -> SkipReason {
        if !row.availability.is_ha()
            && action.is_eligible(&row.status, CloudSqlAvailability::Regional)
        {
            return SkipReason::RequiresHighAvailability;
        }
        SkipReason::Ineligible(row_ref.status_label.clone())
    }

    fn row_ref(row: &Self::Row, context: &GcloudContext, action: Self::Action) -> BatchRowRef {
        BatchRowRef {
            name: row.name.clone(),
            // Cloud SQL's target location is its region.
            location: row.region.clone(),
            status_label: cloud_sql_status_label(&row.status),
            // Shared with the single-row confirmation, so the batch modal
            // shows exactly the call the runtime will make.
            call: cloud_sql_lifecycle_call(context, &row.name, action),
        }
    }
}
