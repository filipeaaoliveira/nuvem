//! The call a mutation will make, named once.
//!
//! Both the argv the runtime runs and the preview the operator reads derive
//! from this, which is what keeps them from drifting apart. It is the same
//! property `compute_lifecycle_argv` held while the transport was a command
//! line, carried somewhere a transport without an argv can still hold it.

use nuvem_domain::gcp::resource_call::ResourceCall;

#[test]
fn a_verb_call_names_its_method_and_its_resource() {
    let call = ResourceCall::new(
        "compute",
        "instances.stop",
        "projects/ledger-prod/zones/europe-west4-b/instances/checkout-api-3",
    );

    assert_eq!(call.summary(), "compute.instances.stop");
    assert_eq!(
        call.preview(),
        "compute.instances.stop zones/europe-west4-b/instances/checkout-api-3",
        "the preview names the method and what it acts on, in one line: the \
         modal declines to draw when it cannot show every line it holds, so \
         each extra line is a terminal height that can no longer confirm"
    );
}

#[test]
fn a_patch_call_names_the_field_it_sets() {
    let call = ResourceCall::new(
        "sql",
        "instances.patch",
        "projects/ledger-prod/instances/orders-db",
    )
    .with_field("settings.activationPolicy", "NEVER");

    assert_eq!(call.summary(), "sql.instances.patch");
    assert_eq!(
        call.preview(),
        "sql.instances.patch instances/orders-db activationPolicy=NEVER",
        "a patch that does not say what it sets tells the operator nothing \
         about what the confirmation is for"
    );
}

#[test]
fn two_calls_that_differ_only_in_their_field_are_not_equal() {
    let stop = ResourceCall::new("sql", "instances.patch", "projects/p/instances/db")
        .with_field("settings.activationPolicy", "NEVER");
    let start = ResourceCall::new("sql", "instances.patch", "projects/p/instances/db")
        .with_field("settings.activationPolicy", "ALWAYS");

    assert_ne!(
        stop, start,
        "start and stop are the same method on the same resource and differ \
         only here; a comparison that missed it would let a test pass while \
         the modal offered the wrong one"
    );
}
