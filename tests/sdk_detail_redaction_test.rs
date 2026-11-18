//! The JSON tab's renderer for the native read path runs every redaction pass
//! on every pane. Nothing else tests that: the CLI path's passes are pinned
//! through `parse_*_detail_json`, which the native mappers never call.
//!
//! Each test plants material in a client model, renders it the way the panes
//! do, and asserts the planted material is gone and `<redacted>` took its
//! place. Planted, not captured: a fixture that already lacked the material
//! would pass over nothing.

use nuvem::services::sdk::detail_json;

const KMS_KEY: &str = "projects/p/locations/l/keyRings/r/cryptoKeys/planted-key";
const PEM: &str = "-----BEGIN CERTIFICATE-----PLANTEDPEM-----END CERTIFICATE-----";

#[test]
fn a_bucket_default_kms_key_is_redacted_in_the_clients_spelling() {
    use google_cloud_storage::model::Bucket;
    use google_cloud_storage::model::bucket::Encryption;

    let bucket = Bucket::new()
        .set_name("projects/_/buckets/planted")
        .set_encryption(Encryption::new().set_default_kms_key(KMS_KEY));
    let json = detail_json(&bucket);

    assert!(
        !json.contains("planted-key"),
        "key material survived:\n{json}"
    );
    assert!(json.contains("<redacted>"), "nothing was redacted:\n{json}");
}

#[test]
fn a_cloud_sql_certificate_is_redacted_by_key_and_by_pem_marker() {
    use google_cloud_sql_v1::model::{DatabaseInstance, SslCert};

    let instance = DatabaseInstance::new()
        .set_name("planted")
        .set_server_ca_cert(SslCert::new().set_cert(PEM));
    let json = detail_json(&instance);

    assert!(!json.contains("PLANTEDPEM"), "PEM body survived:\n{json}");
    assert!(json.contains("<redacted>"), "nothing was redacted:\n{json}");
}

#[test]
fn a_pane_without_secrets_renders_untouched() {
    use google_cloud_compute_v1::model::Instance;

    let mut instance = Instance::new();
    instance.name = Some("web-01".to_string());
    instance.cpu_platform = Some("Intel Cascade Lake".to_string());
    let json = detail_json(&instance);

    assert!(json.contains("\"name\": \"web-01\""));
    assert!(json.contains("\"cpuPlatform\": \"Intel Cascade Lake\""));
    assert!(
        !json.contains("<redacted>"),
        "a clean model was redacted:\n{json}"
    );
}

/// Material in a field the generated model has no word for is redacted too.
///
/// This is the pass's least obvious reach, and the one a reader is most likely
/// to talk themselves out of. `sslCerts` is not a field of `DatabaseInstance`,
/// so the generated deserializer absorbs it into the model's private
/// `_unknown_fields` map rather than rejecting the document -- and the
/// generated serializer writes that map back out again. The array therefore
/// reaches the JSON tab, carrying a PEM and a client key, and `redact_secrets`
/// has to walk it like any other object. Planted through JSON because
/// `_unknown_fields` is private and that is the only way in.
#[test]
fn material_in_a_field_the_model_never_modelled_is_still_redacted() {
    use google_cloud_sql_v1::model::DatabaseInstance;

    let planted = format!(
        r#"{{"name":"planted","sslCerts":[{{"commonName":"planted-cert","cert":"{PEM}","clientKey":"PLANTEDCLIENTKEY"}}]}}"#
    );
    let instance: DatabaseInstance =
        serde_json::from_str(&planted).expect("an unknown field is absorbed, not rejected");
    let json = detail_json(&instance);

    assert!(
        json.contains("planted-cert"),
        "the unmodelled field reaches the pane rather than being dropped, \
         which is exactly why it has to be redacted:\n{json}"
    );
    assert!(!json.contains("PLANTEDPEM"), "PEM body survived:\n{json}");
    assert!(
        !json.contains("PLANTEDCLIENTKEY"),
        "client key survived:\n{json}"
    );
}
