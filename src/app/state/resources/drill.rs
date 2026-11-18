//! The shape both resource drill-downs share.
//!
//! Cloud Storage and BigQuery each browse a hierarchy of levels. Storage has
//! four already — buckets, the objects inside one, and a detail pane for
//! each of those two. BigQuery has three: datasets, the tables inside one, and
//! the column schema of one of those. Each keeps a view enum and an invariant
//! that must hold whenever the view changes, and each enforced that invariant
//! by convention: a private `set_view` and the discipline that nothing else
//! assigns the field.
//!
//! This trait takes over half of that convention. `set_view` is provided, so
//! a caller that goes through it cannot skip the invariant or run it before
//! the store, and a transition added later inherits both without doing
//! anything. The other half stays convention: `store_view` is as visible as
//! the trait, so anything in `resources` could call it and assign a view with
//! no invariant. What holds that up is the written rule that nothing but
//! `set_view` calls `store_view` — not the view field's privacy, which
//! `store_view` is precisely the sanctioned way around.
//!
//! The invariants themselves are deliberately not shared, because they are
//! not the same: Storage syncs its collection's `view_mode`, and BigQuery
//! bumps a drill generation and clears the child collection while syncing no
//! `view_mode` at all.

/// A resource browser with more than one level, and an invariant to restore
/// whenever the level changes.
pub(super) trait DrillHierarchy {
    /// The service's own view enum.
    type View: Copy + PartialEq + std::fmt::Debug;

    /// The view currently stored.
    ///
    /// This exists for the trait and for each service's public view
    /// accessor; code inside the module that owns the field keeps reading it
    /// directly, because the field is private there and the read is the same
    /// `Copy` either way. It is *writes* the convention above is about, and
    /// those go through [`Self::set_view`]; a read cannot break an invariant.
    fn stored_view(&self) -> Self::View;

    /// Assign the view field, and nothing else. Implementations must not put
    /// invariant work here — [`Self::restore_invariant`] is where that runs,
    /// and splitting the two is what lets the provided `set_view` guarantee
    /// the ordering.
    fn store_view(&mut self, view: Self::View);

    /// Restore whatever this service must keep true on arriving at `view`.
    ///
    /// Takes the destination only. Both implementors' invariants are "on
    /// arriving at X, ensure Y"; a `from` nobody reads would be a parameter
    /// invented for a hypothetical.
    fn restore_invariant(&mut self, view: Self::View);

    /// Change the view, running this service's invariant for the view
    /// arrived at.
    ///
    /// Stores first, then restores, so an invariant that reads the view field
    /// sees the destination rather than the view being replaced. It does not
    /// skip when the view is unchanged: re-entering a level is a real
    /// transition for both implementors.
    ///
    /// What this makes structural is the ordering, not exclusivity.
    /// [`Self::store_view`] is as visible as the trait, so anything within
    /// `resources` could call it directly and never restore the invariant;
    /// that it does not remains a convention, held by the written rule that
    /// every transition goes through here.
    fn set_view(&mut self, view: Self::View) {
        self.store_view(view);
        self.restore_invariant(view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeView {
        Root,
        Child,
    }

    #[derive(Default)]
    struct Fake {
        view: Option<FakeView>,
        invariant_ran_for: Vec<FakeView>,
    }

    impl DrillHierarchy for Fake {
        type View = FakeView;

        fn stored_view(&self) -> Self::View {
            self.view.expect("a view was stored")
        }

        fn store_view(&mut self, view: Self::View) {
            self.view = Some(view);
        }

        fn restore_invariant(&mut self, view: Self::View) {
            self.invariant_ran_for.push(view);
        }
    }

    /// What `set_view` does for a caller that uses it: stores the
    /// destination, and runs the invariant exactly once, for that same
    /// destination. Occurrence, arity and argument — not exclusivity, which
    /// nothing here pins.
    #[test]
    fn setting_a_view_runs_the_invariant_for_the_view_arrived_at() {
        let mut fake = Fake::default();

        fake.set_view(FakeView::Child);

        assert_eq!(fake.stored_view(), FakeView::Child);
        assert_eq!(
            fake.invariant_ran_for,
            vec![FakeView::Child],
            "the invariant runs once, for the destination"
        );
    }

    /// Ordering, not just occurrence: the invariant must observe the new view,
    /// because both real implementors branch on it.
    #[test]
    fn the_invariant_observes_the_view_already_stored() {
        #[derive(Default)]
        struct Observer {
            view: Option<FakeView>,
            seen_during_invariant: Option<FakeView>,
        }

        impl DrillHierarchy for Observer {
            type View = FakeView;

            fn stored_view(&self) -> Self::View {
                self.view.expect("a view was stored")
            }

            fn store_view(&mut self, view: Self::View) {
                self.view = Some(view);
            }

            fn restore_invariant(&mut self, _view: Self::View) {
                self.seen_during_invariant = self.view;
            }
        }

        let mut observer = Observer::default();
        observer.set_view(FakeView::Child);

        assert_eq!(
            observer.seen_during_invariant,
            Some(FakeView::Child),
            "store must happen before restore, or an invariant reading \
             self.view sees the view it is replacing"
        );
    }

    /// Re-entering the view already on screen still runs the invariant.
    ///
    /// No caller depends on that today. Storage's invariant is a pure
    /// function of the destination view, so re-running it while already there
    /// changes nothing; and BigQuery's only call that can land on the view it
    /// is already in, `mark_bigquery_datasets_stale_and_clear`, would then
    /// bump a generation over tables the invariant has already cleared. The
    /// test pins the unconditional form anyway, so that adding an equality
    /// guard stays a deliberate decision: BigQuery's invariant is not
    /// idempotent, and a guard added for today's callers would silently
    /// change what a later same-view transition gets.
    ///
    /// The fake starts on a different view so an equality guard added to
    /// `set_view` would fail this assertion, rather than panicking in
    /// `stored_view` on the very first call with nothing stored yet.
    #[test]
    fn setting_the_same_view_again_still_runs_the_invariant() {
        let mut fake = Fake {
            view: Some(FakeView::Child),
            ..Fake::default()
        };

        fake.set_view(FakeView::Root);
        fake.set_view(FakeView::Root);

        assert_eq!(
            fake.invariant_ran_for,
            vec![FakeView::Root, FakeView::Root],
            "no short-circuit on an unchanged view"
        );
    }
}
