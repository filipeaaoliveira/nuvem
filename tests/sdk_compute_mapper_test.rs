//! The Compute Engine mappers: the client's `Instance` turned into the row and
//! the detail the panes render.
//!
//! A mutation sweep reported `map_instance` and `map_instance_detail` as
//! untested, so every field here is planted with a distinct non-default value
//! and asserted exactly — a mapper whose body is replaced wholesale, or whose
//! closure answers `None`/`Some("xyzzy")`/an empty collection, cannot pass.
//! Both sides of every conditional are covered too: absent against present
//! links, a status the vocabulary knows against one it does not, tags absent
//! against present-but-empty, and an empty service-account email against a
//! real one.

use google_cloud_compute_v1::model::AccessConfig;
use google_cloud_compute_v1::model::AttachedDisk;
use google_cloud_compute_v1::model::Instance;
use google_cloud_compute_v1::model::Metadata;
use google_cloud_compute_v1::model::NetworkInterface;
use google_cloud_compute_v1::model::ServiceAccount;
use google_cloud_compute_v1::model::Tags;
use google_cloud_compute_v1::model::attached_disk::Type as DiskType;
use google_cloud_compute_v1::model::instance::Status as ApiStatus;
use google_cloud_compute_v1::model::metadata::Items as MetadataItem;
use nuvem::services::sdk::compute::{map_instance, map_instance_detail};
use nuvem_domain::gcp::resource::{
    ComputeDisk, ComputeInstance, ComputeKeyValue, ComputeNetworkInterface, ResourceStatus,
};

const ZONE_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b";
const MACHINE_TYPE_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/machineTypes/n2-standard-4";
const NETWORK_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/global/networks/payments-vpc";
const SUBNETWORK_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/regions/europe-west1/subnetworks/payments-subnet";
const OPS_NETWORK_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/global/networks/ops-vpc";
const OPS_SUBNETWORK_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/regions/us-east4/subnetworks/ops-subnet";
const BOOT_DISK_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/disks/web-01-boot";
const SCRATCH_DISK_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/disks/web-01-scratch";

fn access_config(nat_ip: Option<&str>) -> AccessConfig {
    let mut config = AccessConfig::new();
    config.nat_ip = nat_ip.map(str::to_string);
    config
}

fn service_account(email: Option<&str>) -> ServiceAccount {
    let mut account = ServiceAccount::new();
    account.email = email.map(str::to_string);
    account
}

fn metadata_item(key: Option<&str>, value: Option<&str>) -> MetadataItem {
    let mut item = MetadataItem::new();
    item.key = key.map(str::to_string);
    item.value = value.map(str::to_string);
    item
}

/// The status a row reads for one wire spelling, which is the only thing these
/// vocabulary tests vary.
fn row_status(wire: &str) -> ResourceStatus {
    let mut instance = Instance::new();
    instance.status = Some(ApiStatus::from(wire));
    map_instance(&instance).status
}

// ---------------------------------------------------------------------------
// map_instance
// ---------------------------------------------------------------------------

#[test]
fn a_row_carries_every_field_of_a_fully_populated_instance() {
    let mut interface = NetworkInterface::new();
    interface.network_ip = Some("10.128.0.7".to_string());
    interface.access_configs = vec![access_config(Some("34.79.1.2"))];

    let mut instance = Instance::new();
    instance.id = Some(8_123_456_789_012_345_678);
    instance.name = Some("web-01".to_string());
    instance.zone = Some(ZONE_LINK.to_string());
    instance.machine_type = Some(MACHINE_TYPE_LINK.to_string());
    instance.status = Some(ApiStatus::Running);
    instance.network_interfaces = vec![interface];

    assert_eq!(
        map_instance(&instance),
        ComputeInstance {
            id: "8123456789012345678".to_string(),
            name: "web-01".to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "n2-standard-4".to_string(),
            internal_ip: Some("10.128.0.7".to_string()),
            external_ip: Some("34.79.1.2".to_string()),
        },
        "every row field comes from the instance it was given"
    );
}

#[test]
fn a_row_from_an_empty_instance_is_empty_rather_than_invented() {
    assert_eq!(
        map_instance(&Instance::new()),
        ComputeInstance {
            id: String::new(),
            name: String::new(),
            zone: String::new(),
            status: ResourceStatus::Unknown(String::new()),
            machine_type: String::new(),
            internal_ip: None,
            external_ip: None,
        },
        "an instance carrying nothing produces empty strings and no addresses"
    );
}

#[test]
fn a_row_shortens_a_self_link_and_leaves_a_bare_name_alone() {
    let mut linked = Instance::new();
    linked.zone = Some(ZONE_LINK.to_string());
    linked.machine_type = Some(MACHINE_TYPE_LINK.to_string());
    let linked = map_instance(&linked);
    assert_eq!(linked.zone, "europe-west1-b");
    assert_eq!(linked.machine_type, "n2-standard-4");

    let mut bare = Instance::new();
    bare.zone = Some("us-central1-a".to_string());
    bare.machine_type = Some("e2-medium".to_string());
    let bare = map_instance(&bare);
    assert_eq!(
        bare.zone, "us-central1-a",
        "a value that is already a name is not shortened further"
    );
    assert_eq!(bare.machine_type, "e2-medium");
}

#[test]
fn a_row_translates_every_status_the_cli_vocabulary_knows() {
    assert_eq!(row_status("RUNNING"), ResourceStatus::Running);
    assert_eq!(row_status("TERMINATED"), ResourceStatus::Stopped);
    assert_eq!(row_status("STOPPED"), ResourceStatus::Stopped);
    assert_eq!(row_status("SUSPENDED"), ResourceStatus::Suspended);
    assert_eq!(row_status("PROVISIONING"), ResourceStatus::Pending);
    assert_eq!(row_status("STAGING"), ResourceStatus::Pending);
    assert_eq!(row_status("STOPPING"), ResourceStatus::Pending);
    assert_eq!(row_status("SUSPENDING"), ResourceStatus::Pending);
    assert_eq!(row_status("RESUMING"), ResourceStatus::Pending);
}

#[test]
fn a_row_keeps_an_unrecognised_status_verbatim() {
    assert_eq!(
        row_status("REPAIRING"),
        ResourceStatus::Unknown("REPAIRING".to_string()),
        "a status the vocabulary does not list is carried through, not dropped"
    );
    assert_eq!(
        row_status("GALACTIC_DRIFT"),
        ResourceStatus::Unknown("GALACTIC_DRIFT".to_string())
    );
}

#[test]
fn a_status_with_no_name_on_the_wire_reads_as_an_empty_unknown() {
    let mut instance = Instance::new();
    // An enum deserialised from an integer the client does not know has no
    // name to read, which is a different absence from no status at all.
    instance.status = Some(ApiStatus::from(9_999));

    assert_eq!(
        map_instance(&instance).status,
        ResourceStatus::Unknown(String::new())
    );
}

#[test]
fn a_row_takes_its_addresses_from_the_first_interface_only() {
    let mut first = NetworkInterface::new();
    first.network_ip = Some("10.128.0.7".to_string());
    first.access_configs = vec![access_config(Some("34.79.1.2"))];
    let mut second = NetworkInterface::new();
    second.network_ip = Some("10.200.0.9".to_string());
    second.access_configs = vec![access_config(Some("35.190.4.5"))];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![first, second];
    let row = map_instance(&instance);

    assert_eq!(row.internal_ip, Some("10.128.0.7".to_string()));
    assert_eq!(
        row.external_ip,
        Some("34.79.1.2".to_string()),
        "the second interface's addresses do not reach the row"
    );
}

#[test]
fn a_row_takes_the_external_address_from_the_first_access_config() {
    let mut interface = NetworkInterface::new();
    interface.network_ip = Some("10.128.0.7".to_string());
    interface.access_configs = vec![
        access_config(Some("34.79.1.2")),
        access_config(Some("35.190.4.5")),
    ];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];

    assert_eq!(
        map_instance(&instance).external_ip,
        Some("34.79.1.2".to_string())
    );
}

#[test]
fn a_row_without_an_access_config_has_an_internal_address_and_no_external_one() {
    let mut interface = NetworkInterface::new();
    interface.network_ip = Some("10.128.0.7".to_string());

    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];
    let row = map_instance(&instance);

    assert_eq!(row.internal_ip, Some("10.128.0.7".to_string()));
    assert_eq!(
        row.external_ip, None,
        "an interface with no access config has no external address"
    );
}

#[test]
fn a_row_interface_without_an_internal_address_reports_none() {
    let mut interface = NetworkInterface::new();
    interface.access_configs = vec![access_config(Some("34.79.1.2"))];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];
    let row = map_instance(&instance);

    assert_eq!(row.internal_ip, None);
    assert_eq!(row.external_ip, Some("34.79.1.2".to_string()));
}

#[test]
fn a_row_access_config_naming_no_address_leaves_the_external_address_absent() {
    // A present access config that holds no NAT address is a different absence
    // from no access config at all, and the row must not confuse "there is a
    // config" with "there is an address".
    let mut interface = NetworkInterface::new();
    interface.network_ip = Some("10.128.0.7".to_string());
    interface.access_configs = vec![access_config(None)];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];
    let row = map_instance(&instance);

    assert_eq!(row.internal_ip, Some("10.128.0.7".to_string()));
    assert_eq!(row.external_ip, None);
}

// ---------------------------------------------------------------------------
// map_instance_detail
// ---------------------------------------------------------------------------

#[test]
fn a_detail_carries_every_scalar_field_of_a_populated_instance() {
    let mut instance = Instance::new();
    instance.name = Some("web-01".to_string());
    instance.zone = Some(ZONE_LINK.to_string());
    instance.machine_type = Some(MACHINE_TYPE_LINK.to_string());
    instance.status = Some(ApiStatus::Terminated);
    instance.creation_timestamp = Some("2024-01-02T03:04:05.000-08:00".to_string());

    let detail = map_instance_detail(&instance);

    assert_eq!(detail.name, "web-01");
    assert_eq!(detail.zone, "europe-west1-b", "the zone link is shortened");
    assert_eq!(
        detail.machine_type, "n2-standard-4",
        "the machine type link is shortened"
    );
    assert_eq!(detail.status, ResourceStatus::Stopped);
    assert_eq!(
        detail.creation_timestamp,
        Some("2024-01-02T03:04:05.000-08:00".to_string()),
        "the creation timestamp passes through untouched"
    );
}

#[test]
fn a_detail_from_an_empty_instance_has_no_scalars_and_no_collections() {
    let detail = map_instance_detail(&Instance::new());

    assert_eq!(detail.name, "");
    assert_eq!(detail.zone, "");
    assert_eq!(detail.machine_type, "");
    assert_eq!(detail.status, ResourceStatus::Unknown(String::new()));
    assert_eq!(detail.creation_timestamp, None);
    assert!(detail.network_interfaces.is_empty());
    assert!(detail.tags.is_empty());
    assert!(detail.service_accounts.is_empty());
    assert!(detail.disks.is_empty());
    assert!(detail.labels.is_empty());
    assert!(detail.metadata.is_empty());
}

#[test]
fn a_detail_keeps_an_unrecognised_status_verbatim() {
    let mut instance = Instance::new();
    instance.status = Some(ApiStatus::from("GALACTIC_DRIFT"));

    assert_eq!(
        map_instance_detail(&instance).status,
        ResourceStatus::Unknown("GALACTIC_DRIFT".to_string())
    );
}

#[test]
fn a_detail_status_with_no_name_on_the_wire_reads_as_an_empty_unknown() {
    let mut instance = Instance::new();
    // Deserialised from an integer the client does not know: a status is
    // present, but there is no name to translate.
    instance.status = Some(ApiStatus::from(9_999));

    assert_eq!(
        map_instance_detail(&instance).status,
        ResourceStatus::Unknown(String::new())
    );
}

#[test]
fn detail_interfaces_keep_their_order_and_shorten_their_links() {
    let mut first = NetworkInterface::new();
    first.name = Some("nic0".to_string());
    first.network = Some(NETWORK_LINK.to_string());
    first.subnetwork = Some(SUBNETWORK_LINK.to_string());
    first.network_ip = Some("10.128.0.7".to_string());
    first.access_configs = vec![access_config(Some("34.79.1.2"))];

    let mut second = NetworkInterface::new();
    second.name = Some("nic1".to_string());
    second.network = Some(OPS_NETWORK_LINK.to_string());
    second.subnetwork = Some(OPS_SUBNETWORK_LINK.to_string());
    second.network_ip = Some("10.200.0.9".to_string());
    second.access_configs = vec![access_config(Some("35.190.4.5"))];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![first, second];

    assert_eq!(
        map_instance_detail(&instance).network_interfaces,
        vec![
            ComputeNetworkInterface {
                name: Some("nic0".to_string()),
                network: Some("payments-vpc".to_string()),
                subnetwork: Some("payments-subnet".to_string()),
                subnetwork_region: Some("europe-west1".to_string()),
                internal_ip: Some("10.128.0.7".to_string()),
                external_ip: Some("34.79.1.2".to_string()),
            },
            ComputeNetworkInterface {
                name: Some("nic1".to_string()),
                network: Some("ops-vpc".to_string()),
                subnetwork: Some("ops-subnet".to_string()),
                subnetwork_region: Some("us-east4".to_string()),
                internal_ip: Some("10.200.0.9".to_string()),
                external_ip: Some("35.190.4.5".to_string()),
            },
        ],
        "both interfaces are mapped, in the order the API returned them"
    );
}

#[test]
fn an_interface_without_links_or_addresses_reports_them_absent() {
    let interface = NetworkInterface::new();
    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];

    assert_eq!(
        map_instance_detail(&instance).network_interfaces,
        vec![ComputeNetworkInterface {
            name: None,
            network: None,
            subnetwork: None,
            subnetwork_region: None,
            internal_ip: None,
            external_ip: None,
        }],
        "an interface that names nothing is carried as an interface naming nothing"
    );
}

#[test]
fn a_subnetwork_link_naming_no_region_leaves_the_region_absent() {
    let mut interface = NetworkInterface::new();
    interface.subnetwork = Some("projects/acme-prod/subnetworks/flat-subnet".to_string());
    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];

    let interfaces = map_instance_detail(&instance).network_interfaces;
    let mapped = &interfaces[0];
    assert_eq!(
        mapped.subnetwork,
        Some("flat-subnet".to_string()),
        "the subnet name is still read"
    );
    assert_eq!(
        mapped.subnetwork_region, None,
        "a link nested differently answers None rather than the wrong segment"
    );
}

#[test]
fn a_subnetwork_link_ending_at_the_region_marker_leaves_the_region_absent() {
    let mut interface = NetworkInterface::new();
    interface.subnetwork = Some("projects/acme-prod/regions/".to_string());
    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];

    assert_eq!(
        map_instance_detail(&instance).network_interfaces[0].subnetwork_region,
        None,
        "an empty segment after the marker is not a region"
    );
}

#[test]
fn a_detail_interface_takes_the_external_address_from_the_first_access_config() {
    let mut with_two = NetworkInterface::new();
    with_two.name = Some("nic0".to_string());
    with_two.access_configs = vec![
        access_config(Some("34.79.1.2")),
        access_config(Some("35.190.4.5")),
    ];
    let mut without_any = NetworkInterface::new();
    without_any.name = Some("nic1".to_string());

    let mut instance = Instance::new();
    instance.network_interfaces = vec![with_two, without_any];
    let interfaces = map_instance_detail(&instance).network_interfaces;

    assert_eq!(interfaces[0].external_ip, Some("34.79.1.2".to_string()));
    assert_eq!(
        interfaces[1].external_ip, None,
        "an interface with no access config has no external address"
    );
}

#[test]
fn a_detail_interface_access_config_naming_no_address_reports_none() {
    let mut interface = NetworkInterface::new();
    interface.name = Some("nic0".to_string());
    interface.network_ip = Some("10.128.0.7".to_string());
    interface.access_configs = vec![access_config(None)];

    let mut instance = Instance::new();
    instance.network_interfaces = vec![interface];
    let interfaces = map_instance_detail(&instance).network_interfaces;

    assert_eq!(interfaces[0].internal_ip, Some("10.128.0.7".to_string()));
    assert_eq!(
        interfaces[0].external_ip, None,
        "a config that holds no NAT address is read as no external address"
    );
}

#[test]
fn tags_are_carried_in_order_when_the_instance_has_them() {
    let mut tags = Tags::new();
    tags.items = vec!["web-server".to_string(), "allow-ssh".to_string()];
    let mut instance = Instance::new();
    instance.tags = Some(tags);

    assert_eq!(
        map_instance_detail(&instance).tags,
        vec!["web-server".to_string(), "allow-ssh".to_string()],
        "tags keep the order the API returned"
    );
}

#[test]
fn tags_absent_and_tags_present_but_empty_both_mean_none() {
    let absent = map_instance_detail(&Instance::new());
    assert!(absent.tags.is_empty(), "no tags block means no tags");

    let mut instance = Instance::new();
    instance.tags = Some(Tags::new());
    assert!(
        map_instance_detail(&instance).tags.is_empty(),
        "an empty tags block means no tags either"
    );
}

#[test]
fn service_accounts_are_emails_in_order_with_the_empty_ones_dropped() {
    let mut instance = Instance::new();
    instance.service_accounts = vec![
        service_account(Some("runner@acme-prod.iam.gserviceaccount.com")),
        service_account(Some("")),
        service_account(None),
        service_account(Some("builder@acme-prod.iam.gserviceaccount.com")),
    ];

    assert_eq!(
        map_instance_detail(&instance).service_accounts,
        vec![
            "runner@acme-prod.iam.gserviceaccount.com".to_string(),
            "builder@acme-prod.iam.gserviceaccount.com".to_string(),
        ],
        "an entry with no email identifies nothing and is not carried"
    );
}

#[test]
fn an_instance_whose_only_service_account_has_an_empty_email_carries_none() {
    let mut instance = Instance::new();
    instance.service_accounts = vec![service_account(Some(""))];

    assert!(
        map_instance_detail(&instance).service_accounts.is_empty(),
        "an empty email must not render as a selector that matches everything"
    );
}

#[test]
fn disks_keep_their_order_and_carry_the_attachment_facts() {
    let mut boot = AttachedDisk::new();
    boot.device_name = Some("persistent-disk-0".to_string());
    boot.source = Some(BOOT_DISK_LINK.to_string());
    boot.boot = Some(true);
    boot.auto_delete = Some(true);
    boot.r#type = Some(DiskType::Persistent);
    boot.disk_size_gb = Some(100);

    let mut scratch = AttachedDisk::new();
    scratch.device_name = Some("local-ssd-0".to_string());
    scratch.source = Some(SCRATCH_DISK_LINK.to_string());
    scratch.boot = Some(false);
    scratch.auto_delete = Some(false);
    scratch.r#type = Some(DiskType::Scratch);
    scratch.disk_size_gb = Some(375);

    let mut instance = Instance::new();
    instance.disks = vec![boot, scratch];

    assert_eq!(
        map_instance_detail(&instance).disks,
        vec![
            ComputeDisk {
                name: Some("persistent-disk-0".to_string()),
                source: Some("web-01-boot".to_string()),
                boot: true,
                auto_delete: true,
                disk_type: Some("PERSISTENT".to_string()),
                size_gb: Some("100".to_string()),
            },
            ComputeDisk {
                name: Some("local-ssd-0".to_string()),
                source: Some("web-01-scratch".to_string()),
                boot: false,
                auto_delete: false,
                disk_type: Some("SCRATCH".to_string()),
                size_gb: Some("375".to_string()),
            },
        ],
        "each attachment's own facts are carried, in order"
    );
}

#[test]
fn a_disk_that_states_nothing_is_neither_boot_nor_auto_deleted() {
    let mut instance = Instance::new();
    instance.disks = vec![AttachedDisk::new()];

    assert_eq!(
        map_instance_detail(&instance).disks,
        vec![ComputeDisk {
            name: None,
            source: None,
            boot: false,
            auto_delete: false,
            disk_type: None,
            size_gb: None,
        }],
        "an unstated flag reads as false, and an unstated type or size as absent"
    );
}

#[test]
fn a_disk_type_with_no_name_on_the_wire_is_absent_rather_than_empty() {
    let mut disk = AttachedDisk::new();
    disk.device_name = Some("persistent-disk-0".to_string());
    // An enum deserialised from an unknown integer has no name to read.
    disk.r#type = Some(DiskType::from(9_999));
    let mut instance = Instance::new();
    instance.disks = vec![disk];

    assert_eq!(map_instance_detail(&instance).disks[0].disk_type, None);
}

#[test]
fn labels_become_key_value_pairs() {
    let mut instance = Instance::new();
    instance
        .labels
        .insert("env".to_string(), "production".to_string());
    instance
        .labels
        .insert("team".to_string(), "payments".to_string());

    // The wire form is a map, so the mapper's output order is the map's; the
    // pairs themselves are what this pins.
    let mut labels = map_instance_detail(&instance).labels;
    labels.sort_by(|left, right| left.key.cmp(&right.key));

    assert_eq!(
        labels,
        vec![
            ComputeKeyValue {
                key: "env".to_string(),
                value: "production".to_string(),
            },
            ComputeKeyValue {
                key: "team".to_string(),
                value: "payments".to_string(),
            },
        ]
    );
}

#[test]
fn metadata_items_keep_their_order_and_a_missing_half_reads_as_empty() {
    let mut metadata = Metadata::new();
    metadata.items = vec![
        metadata_item(Some("startup-script"), Some("#!/bin/sh\necho hello")),
        metadata_item(Some("enable-oslogin"), None),
        metadata_item(None, Some("orphaned-value")),
    ];
    let mut instance = Instance::new();
    instance.metadata = Some(metadata);

    assert_eq!(
        map_instance_detail(&instance).metadata,
        vec![
            ComputeKeyValue {
                key: "startup-script".to_string(),
                value: "#!/bin/sh\necho hello".to_string(),
            },
            ComputeKeyValue {
                key: "enable-oslogin".to_string(),
                value: String::new(),
            },
            ComputeKeyValue {
                key: String::new(),
                value: "orphaned-value".to_string(),
            },
        ],
        "an entry with only one half is kept, with the other half empty"
    );
}

#[test]
fn metadata_absent_and_metadata_without_items_both_read_as_none() {
    assert!(map_instance_detail(&Instance::new()).metadata.is_empty());

    let mut instance = Instance::new();
    instance.metadata = Some(Metadata::new());
    assert!(
        map_instance_detail(&instance).metadata.is_empty(),
        "a metadata block holding no items is no metadata"
    );
}

#[test]
fn the_raw_json_renders_the_instance_the_client_returned() {
    let mut instance = Instance::new();
    instance.name = Some("web-01".to_string());
    instance.cpu_platform = Some("Intel Cascade Lake".to_string());

    let raw = map_instance_detail(&instance).raw_json;

    assert!(
        raw.contains("\"name\": \"web-01\""),
        "the JSON tab shows the instance itself:\n{raw}"
    );
    assert!(
        raw.contains("\"cpuPlatform\": \"Intel Cascade Lake\""),
        "fields the detail struct does not model still reach the JSON tab:\n{raw}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&raw).is_ok(),
        "the JSON tab is given JSON:\n{raw}"
    );
}
