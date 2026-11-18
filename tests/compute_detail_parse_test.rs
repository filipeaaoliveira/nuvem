use nuvem::services::resources::compute::parse_instance_detail_json;
use nuvem_domain::gcp::resource::ResourceStatus;

#[test]
fn parses_gce_instance_detail_tabs() {
    let detail = parse_instance_detail_json(include_str!("fixtures/gce_instance_detail.json"))
        .expect("detail fixture should parse");

    assert_eq!(detail.name, "api-dev-1");
    assert_eq!(detail.zone, "europe-west1-b");
    assert_eq!(detail.status, ResourceStatus::Running);
    assert_eq!(detail.machine_type, "e2-medium");
    assert_eq!(
        detail.creation_timestamp.as_deref(),
        Some("2026-07-03T10:11:12.000-07:00")
    );
    assert_eq!(detail.network_interfaces.len(), 1);
    assert_eq!(detail.network_interfaces[0].name.as_deref(), Some("nic0"));
    assert_eq!(
        detail.network_interfaces[0].network.as_deref(),
        Some("default")
    );
    assert_eq!(
        detail.network_interfaces[0].subnetwork.as_deref(),
        Some("default")
    );
    // The captured document's subnetwork self-link is
    // `.../projects/dev-project/regions/europe-west1/subnetworks/default`, and
    // the region half of it is what identifies the subnet: an auto-mode
    // network names every one of its subnets `default`, so the tail alone
    // names as many rows as the project has regions.
    assert_eq!(
        detail.network_interfaces[0].subnetwork_region.as_deref(),
        Some("europe-west1")
    );
    // Worth stating because this fixture is exactly the case that makes the
    // wrong fix look right: the instance is in `europe-west1-b`, so stripping
    // the zone's last segment would also have produced `europe-west1` here.
    // The region is read from the link the API returned instead, which is the
    // only place it is actually asserted.
    assert_eq!(detail.zone, "europe-west1-b");
    // The network link is `.../projects/dev-project/global/networks/default` —
    // `global`, not a region — so there is no counterpart to keep for it. A
    // network is global within its project, which is why a
    // jump can be keyed on the name alone.
    assert!(
        include_str!("fixtures/gce_instance_detail.json").contains("/global/networks/default"),
        "the captured network link must stay global for the claim above to hold"
    );
    assert_eq!(
        detail.network_interfaces[0].internal_ip.as_deref(),
        Some("10.0.0.4")
    );
    assert_eq!(
        detail.network_interfaces[0].external_ip.as_deref(),
        Some("34.10.10.10")
    );
    assert_eq!(detail.disks.len(), 2);
    assert_eq!(detail.disks[0].name.as_deref(), Some("api-dev-1"));
    assert!(detail.disks[0].boot);
    assert!(detail.disks[0].auto_delete);
    assert_eq!(detail.disks[1].size_gb.as_deref(), Some("100"));
    assert!(
        detail
            .labels
            .iter()
            .any(|entry| entry.key == "env" && entry.value == "dev")
    );
    assert!(
        detail
            .metadata
            .iter()
            .any(|entry| entry.key == "owner" && entry.value == "platform")
    );
    assert!(detail.raw_json.contains("\"name\": \"api-dev-1\""));
}

/// A subnetwork link that names no region leaves the region absent rather than
/// guessing one.
///
/// CONSTRUCTED, not captured: every subnetwork link `compute instances
/// describe` has returned in this project carries `/regions/<r>/`, so there is
/// no captured document with this shape and this asserts what the parser does
/// when it meets one — a defensive verdict, not an observed API behaviour.
///
/// The verdict matters because it is the safe one: an absent
/// `{subnetwork_region}` makes a jump keyed on it produce no offer at all,
/// where a guessed region would produce an offer that lands on the wrong row.
#[test]
fn a_subnetwork_link_with_no_region_segment_yields_no_region() {
    let json = r#"{
        "name": "api-dev-1",
        "zone": "https://www.googleapis.com/compute/v1/projects/p/zones/europe-west1-b",
        "status": "RUNNING",
        "machineType": "https://www.googleapis.com/compute/v1/projects/p/zones/europe-west1-b/machineTypes/e2-medium",
        "networkInterfaces": [
            {
                "name": "nic0",
                "network": "https://www.googleapis.com/compute/v1/projects/p/global/networks/default",
                "subnetwork": "projects/p/subnetworks/default"
            }
        ]
    }"#;

    let detail = parse_instance_detail_json(json).expect("constructed detail should parse");
    let nic = &detail.network_interfaces[0];
    // The name still shortens, so the field a label renders is unaffected.
    assert_eq!(nic.subnetwork.as_deref(), Some("default"));
    assert_eq!(nic.subnetwork_region, None);
}

/// The region is read from the link's own structure, not from where it happens
/// to sit.
///
/// CONSTRUCTED. A link one level deeper than the captured shape still answers
/// with the segment after `regions`, where counting back from the end would
/// answer with whatever landed in that position. The two agree on every
/// captured document, which is exactly why the wrong one would go unnoticed.
#[test]
fn the_subnetwork_region_is_the_segment_after_regions_not_a_fixed_position() {
    let json = r#"{
        "name": "api-dev-1",
        "zone": "https://www.googleapis.com/compute/v1/projects/p/zones/europe-west1-b",
        "status": "RUNNING",
        "machineType": "e2-medium",
        "networkInterfaces": [
            {
                "name": "nic0",
                "subnetwork": "https://www.googleapis.com/compute/v1/projects/p/regions/europe-west4/subnetworks/shared/aliases/x"
            }
        ]
    }"#;

    let detail = parse_instance_detail_json(json).expect("constructed detail should parse");
    assert_eq!(
        detail.network_interfaces[0].subnetwork_region.as_deref(),
        Some("europe-west4")
    );
}

// ---------------------------------------------------------------------------
// The two things a firewall rule can select an instance by
// ---------------------------------------------------------------------------

/// CAPTURED. `tests/fixtures/gce_instance_detail.json` carries neither a
/// `tags` key nor a `serviceAccounts` key, so both read as empty.
///
/// That is not a gap in the fixture, it is what a plain instance looks like —
/// and it is the case that matters most for matching, because an instance with
/// no tags and no service accounts is selected *only* by rules that name no
/// target at all. Those are six of the seven rules in the firewall fixture.
#[test]
fn a_captured_instance_carries_no_tags_and_no_service_accounts() {
    let detail = parse_instance_detail_json(include_str!("fixtures/gce_instance_detail.json"))
        .expect("detail fixture should parse");

    assert!(detail.tags.is_empty());
    assert!(detail.service_accounts.is_empty());
    // Stated so the claim above is checked rather than asserted: if a future
    // capture grows either key, this fails and the two assertions become real
    // assertions about content instead of about absence.
    let raw = include_str!("fixtures/gce_instance_detail.json");
    assert!(
        !raw.contains("\"tags\""),
        "the capture must still omit tags"
    );
    assert!(
        !raw.contains("\"serviceAccounts\""),
        "the capture must still omit serviceAccounts"
    );
}

/// CONSTRUCTED, and this is where the capture gap actually bites: no
/// instance this account can reach carries either key, so the *spellings*
/// below — `tags.items` and `serviceAccounts[].email` — rest on Google's
/// published `compute/v1/instances` reference rather than on bytes the API
/// sent.
///
/// What would confirm them is one `gcloud compute instances describe` of a
/// tagged instance running as a service account. Until then, a wrong spelling
/// would read as an instance with no tags — which is a *silent* wrong answer
/// for matching, since it makes tag-scoped rules disappear rather than error.
#[test]
fn tags_and_service_accounts_are_read_from_their_documented_shapes() {
    let json = r#"{
        "name": "api-dev-1",
        "zone": "https://www.googleapis.com/compute/v1/projects/p/zones/europe-west1-b",
        "status": "RUNNING",
        "machineType": "e2-medium",
        "tags": {
            "items": ["web-server", "bastion"],
            "fingerprint": "abcd1234"
        },
        "serviceAccounts": [
            {
                "email": "workload@example.iam.gserviceaccount.com",
                "scopes": ["https://www.googleapis.com/auth/cloud-platform"]
            }
        ]
    }"#;

    let detail = parse_instance_detail_json(json).expect("constructed detail should parse");
    assert_eq!(detail.tags, vec!["web-server", "bastion"]);
    // The email only. The scopes beside it say what the account may do, which
    // is an authorization question a firewall rule does not ask.
    assert_eq!(
        detail.service_accounts,
        vec!["workload@example.iam.gserviceaccount.com"]
    );
}

/// SYNTHETIC — a service account entry with no email, which the API does not
/// send.
///
/// It is dropped rather than carried as an empty string. An empty entry
/// identifies nothing, and keeping it would let a rule whose selector rendered
/// empty appear to match an instance it does not govern.
#[test]
fn a_service_account_with_no_email_is_dropped_rather_than_carried_empty() {
    let json = r#"{
        "name": "api-dev-1",
        "zone": "europe-west1-b",
        "status": "RUNNING",
        "machineType": "e2-medium",
        "serviceAccounts": [{"scopes": []}, {"email": "real@example.iam.gserviceaccount.com"}]
    }"#;

    let detail = parse_instance_detail_json(json).expect("synthetic detail should parse");
    assert_eq!(
        detail.service_accounts,
        vec!["real@example.iam.gserviceaccount.com"]
    );
}

/// An absent `tags` key and a present-but-empty one give the same answer, and
/// that fold is deliberate rather than incidental.
///
/// SYNTHETIC for the empty half — `instances describe` omits the key rather
/// than sending `{"items": []}`. It is asserted anyway because the parser now
/// *sees* the difference (the raw field is an `Option`, not a defaulted
/// struct) and chooses to fold it, so the choice needs a test that fails if
/// someone changes it by accident.
///
/// Why it is the right fold *here*: a full `describe` response carries every
/// field the resource has, so an absent `tags` means the instance has none.
/// It would not mean that for a partial projection, or for a response a
/// permission filter had trimmed — GCP reports an inaccessible project as
/// "not found" rather than as denied, so absent-versus-withheld is not always
/// visible from the wire. If nuvem ever reads instances through a projection,
/// this is the assertion that has to change first.
#[test]
fn an_absent_tags_key_and_an_empty_one_both_mean_the_instance_carries_none() {
    let absent = r#"{
        "name": "api-dev-1",
        "zone": "europe-west1-b",
        "status": "RUNNING",
        "machineType": "e2-medium"
    }"#;
    let empty = r#"{
        "name": "api-dev-1",
        "zone": "europe-west1-b",
        "status": "RUNNING",
        "machineType": "e2-medium",
        "tags": {"items": []},
        "serviceAccounts": []
    }"#;

    let absent = parse_instance_detail_json(absent).expect("absent-key document parses");
    let empty = parse_instance_detail_json(empty).expect("empty-list document parses");

    assert!(absent.tags.is_empty());
    assert!(absent.service_accounts.is_empty());
    assert_eq!(absent.tags, empty.tags);
    assert_eq!(absent.service_accounts, empty.service_accounts);
}
