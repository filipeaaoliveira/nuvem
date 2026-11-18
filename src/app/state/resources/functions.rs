use super::*;

/// Marker type unifying the Cloud Functions service behind [`ResourceService`].
/// Never instantiated.
pub enum CloudFunctions {}

impl ResourceService for CloudFunctions {
    type Row = CloudFunction;
    /// No detail pane in this slice, so there is nothing for a describe to
    /// fill and no tab to land on. The unit types make that the type rather
    /// than a placeholder struct the renderer could be handed.
    type Detail = ();
    type SortField = CloudFunctionSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::CloudFunctions;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.functions
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.functions
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        function_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the function table.
    fn sample_row() -> Self::Row {
        CloudFunction {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            region: "europe-west1".to_string(),
            state: CloudFunctionState::Active,
            environment: CloudFunctionEnvironment::Gen2,
            url: None,
            update_time: None,
            labels: Vec::new(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_functions(left, right, sort_field, direction)
    }

    /// Cloud Functions is read-only here — no lifecycle actions, so no marks.
    /// This exists to keep [`ResourceService`] total.
    ///
    /// A function's name is unique within a region, not within a project:
    /// `projects/p/locations/europe-west1/functions/api` and its
    /// `us-central1` namesake are two functions. The region is therefore part
    /// of the identity, not decoration.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.region.clone(),
        }
    }
}

/// The sortable columns of the functions table.
///
/// One variant per *sortable* catalogue column. The catalogue has six columns
/// and five of them sort: `url` does not, for the reason recorded beside
/// `CLOUD_FUNCTION_COLUMNS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudFunctionSortField {
    Name,
    Region,
    State,
    Generation,
    Updated,
}

impl CloudFunctionSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Region => "Region",
            Self::State => "State",
            Self::Generation => "Generation",
            Self::Updated => "Updated",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Region),
            2 => Some(Self::State),
            3 => Some(Self::Generation),
            4 => Some(Self::Updated),
            // 5 is `url`, the one catalogue column with no sort field.
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Region => 1,
            Self::State => 2,
            Self::Generation => 3,
            Self::Updated => 4,
        }
    }
}

impl ResourcesState {
    pub fn cloud_functions(&self) -> &[CloudFunction] {
        &self.functions.rows
    }

    pub fn visible_cloud_functions(&self) -> Vec<&CloudFunction> {
        let filter = self.functions.filter_applied.filter();
        let mut rows = self
            .functions
            .rows
            .iter()
            .filter(|function| function_matches_filter(function, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_functions(
                left,
                right,
                self.functions.sort_field,
                self.functions.sort_direction,
            )
        });
        rows
    }

    pub fn visible_cloud_function_count(&self) -> usize {
        let filter = self.functions.filter_applied.filter();
        self.functions
            .rows
            .iter()
            .filter(|function| function_matches_filter(function, filter))
            .count()
    }

    pub fn selected_cloud_function(&self) -> Option<&CloudFunction> {
        self.visible_cloud_functions()
            .get(self.functions.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the function it
    /// was already on where that function is still listed.
    pub fn set_cloud_functions(&mut self, context: GcloudContext, functions: Vec<CloudFunction>) {
        let previously_selected = self
            .selected_cloud_function()
            .map(|function| (function.name.clone(), function.region.clone()));

        self.functions.context = Some(context);
        self.functions.rows = functions;
        self.functions.stale = false;
        self.functions.last_problem = None;
        self.functions.last_diagnostic = None;
        self.functions.clear_load_phase();

        self.functions.selected_visible = previously_selected
            .as_ref()
            .and_then(|(name, region)| {
                self.visible_cloud_functions()
                    .iter()
                    .position(|function| &function.name == name && &function.region == region)
            })
            .unwrap_or_else(|| {
                self.functions
                    .selected_visible
                    .min(self.visible_cloud_functions().len().saturating_sub(1))
            });
    }

    pub fn mark_cloud_functions_stale_and_clear(&mut self) {
        self.functions.filter_draft = self.functions.filter_applied.source().to_string();
        self.functions.filter_input_active = false;
        self.functions.rows.clear();
        self.functions.context = None;
        self.functions.selected_visible = 0;
        self.functions.stale = true;
    }

    pub fn cloud_functions_problem(&self) -> Option<&DiagnosticProblem> {
        self.functions.last_problem.as_ref()
    }
}

/// The values a functions filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// State and generation contribute their *display labels* — the same strings
/// the cells carry, `Unrecognized`'s raw value included, since that is what
/// its label returns. No `{:?}` shell reaches this list; see the compute
/// service's `searchable_fields` for what that cost. The never-drop-a-row
/// premise holds trivially here because this service pushes no regex clause
/// down at all (`FILTER_FIELDS` in `crate::services::resources::functions` is
/// empty), so every one of these values is matched client-side against a row
/// that was never narrowed server-side.
///
/// Labels are included as `key=value` because that is how an operator thinks
/// of them and how the label filter grammar spells them, so a plain substring
/// search for `env=prod` finds the same rows the grammar would.
fn function_searchable_fields(function: &CloudFunction) -> Vec<String> {
    let mut fields = vec![
        function.name.clone(),
        function.region.clone(),
        function.state.label().to_string(),
        function.environment.label().to_string(),
        function.update_time.clone().unwrap_or_default(),
        function.url.clone().unwrap_or_default(),
    ];
    fields.extend(
        function
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn function_matches_filter(function: &CloudFunction, filter: &CompiledFilter) -> bool {
    filter.matches(&function_searchable_fields(function))
}

fn compare_functions(
    left: &CloudFunction,
    right: &CloudFunction,
    sort_field: CloudFunctionSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        CloudFunctionSortField::Name => left.name.cmp(&right.name),
        CloudFunctionSortField::Region => left.region.cmp(&right.region),
        // By the label, which is what the column paints and what the sort
        // arrow above it claims to have ordered. The API values would sort
        // `DETACH_FAILED` before `DETACHING` while the screen reads the other
        // way round, and `GEN_1`/`GEN_2` appear on screen nowhere at all.
        CloudFunctionSortField::State => left.state.label().cmp(&right.state.label()),
        CloudFunctionSortField::Generation => {
            left.environment.label().cmp(&right.environment.label())
        }
        // RFC 3339 with a `Z` offset sorts chronologically as text. An
        // unreported time is the empty string, so those rows group together
        // rather than scattering through the ordering.
        CloudFunctionSortField::Updated => left
            .update_time
            .as_deref()
            .unwrap_or_default()
            .cmp(right.update_time.as_deref().unwrap_or_default()),
    };
    // Region breaks a name tie rather than the name breaking itself: two
    // functions can share a name across regions, so name alone is not a total
    // order over this row set.
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.region.cmp(&right.region));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
