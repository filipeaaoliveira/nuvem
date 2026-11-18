use crate::app::service_registry::{ServiceId, matching_service_ids};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigatorFocus {
    Navigator,
    Resources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceNavigatorState {
    selected: ServiceId,
    filter_draft: String,
    filter_applied: String,
    filter_input_active: bool,
    sidebar_visible: bool,
    focus: NavigatorFocus,
}

impl Default for ServiceNavigatorState {
    fn default() -> Self {
        Self {
            selected: ServiceId::ComputeInstances,
            filter_draft: String::new(),
            filter_applied: String::new(),
            filter_input_active: false,
            sidebar_visible: true,
            focus: NavigatorFocus::Resources,
        }
    }
}

impl ServiceNavigatorState {
    pub fn visible_ids(&self) -> Vec<ServiceId> {
        matching_service_ids(&self.filter_applied)
    }

    pub fn selected_id(&self) -> ServiceId {
        self.selected
    }

    pub fn filter_draft(&self) -> &str {
        &self.filter_draft
    }

    pub fn filter_applied(&self) -> &str {
        &self.filter_applied
    }

    pub fn is_filter_input_active(&self) -> bool {
        self.filter_input_active
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn focus(&self) -> NavigatorFocus {
        self.focus
    }

    pub fn select(&mut self, id: ServiceId) {
        self.selected = id;
    }

    pub fn select_next(&mut self) {
        let visible = self.visible_ids();
        let Some(last) = visible.len().checked_sub(1) else {
            return;
        };

        let current = visible.iter().position(|id| *id == self.selected);
        let next = current.map(|index| (index + 1).min(last)).unwrap_or(0);
        self.selected = visible[next];
    }

    pub fn select_previous(&mut self) {
        let visible = self.visible_ids();
        if visible.is_empty() {
            return;
        }

        let current = visible
            .iter()
            .position(|id| *id == self.selected)
            .unwrap_or(0);
        self.selected = visible[current.saturating_sub(1)];
    }

    pub fn start_filter(&mut self) {
        self.filter_draft = self.filter_applied.clone();
        self.filter_input_active = true;
    }

    pub fn append_filter_char(&mut self, value: char) {
        if !value.is_control() {
            self.filter_draft.push(value);
        }
    }

    pub fn backspace_filter(&mut self) {
        self.filter_draft.pop();
    }

    pub fn apply_filter(&mut self) {
        self.filter_applied = self.filter_draft.clone();
        self.filter_input_active = false;
        self.clamp_selection();
    }

    pub fn cancel_filter(&mut self) {
        self.filter_draft = self.filter_applied.clone();
        self.filter_input_active = false;
        self.clamp_selection();
    }

    /// Directly set sidebar visibility (e.g. applying a reloaded config's
    /// `sidebar_visible` default). Hiding the sidebar moves focus off it.
    pub fn set_sidebar_visible(&mut self, visible: bool) {
        self.sidebar_visible = visible;
        if !self.sidebar_visible {
            self.focus = NavigatorFocus::Resources;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus = NavigatorFocus::Resources;
        }
    }

    pub fn toggle_focus(&mut self) {
        if !self.sidebar_visible {
            self.focus = NavigatorFocus::Resources;
            return;
        }

        self.focus = match self.focus {
            NavigatorFocus::Navigator => NavigatorFocus::Resources,
            NavigatorFocus::Resources => NavigatorFocus::Navigator,
        };
    }

    fn clamp_selection(&mut self) {
        let visible = self.visible_ids();
        if !visible.contains(&self.selected)
            && let Some(first) = visible.first().copied()
        {
            self.selected = first;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `clamp_selection` is what keeps the navigator's cursor on a row that
    /// still exists. Nothing reached it, because every other test selects a
    /// service that is visible; it only runs when the visible set shrinks out
    /// from under the selection, which is what applying a filter does.
    #[test]
    fn a_filter_that_hides_the_selection_moves_it_to_the_first_match() {
        let mut state = ServiceNavigatorState::default();
        state.select(ServiceId::ComputeInstances);

        // A filter matching a different service, so the current selection is
        // no longer one of the visible ids.
        for character in "bigquery".chars() {
            state.append_filter_char(character);
        }
        state.apply_filter();

        let visible = state.visible_ids();
        assert!(
            !visible.contains(&ServiceId::ComputeInstances),
            "the fixture must actually hide the selection: {visible:?}"
        );
        assert_eq!(
            state.selected_id(),
            visible[0],
            "a hidden selection must land on the first visible row"
        );
    }

    /// A filter matching nothing leaves the selection alone: there is no
    /// first row to move to, and resetting to some default would lose the
    /// operator's place the moment they mistype.
    #[test]
    fn a_filter_matching_nothing_leaves_the_selection_where_it_was() {
        let mut state = ServiceNavigatorState::default();
        state.select(ServiceId::ComputeInstances);

        for character in "no-such-service-anywhere".chars() {
            state.append_filter_char(character);
        }
        state.apply_filter();

        assert!(
            state.visible_ids().is_empty(),
            "the fixture must match none"
        );
        assert_eq!(state.selected_id(), ServiceId::ComputeInstances);
    }

    /// The other side of the same `!`: a selection the filter still shows
    /// must be left exactly where it is, not reset to the top.
    #[test]
    fn a_filter_that_still_shows_the_selection_leaves_it_alone() {
        let mut state = ServiceNavigatorState::default();
        let visible = ServiceNavigatorState::default().visible_ids();
        let second = visible[1];
        state.select(second);

        state.apply_filter();

        assert_eq!(
            state.selected_id(),
            second,
            "an unfiltered apply must not move a visible selection"
        );
    }
}
