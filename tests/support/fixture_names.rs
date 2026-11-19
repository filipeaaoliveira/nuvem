//! What a demo document or a demo screen may name.
//!
//! Every project is `dev-project` (or `prod-project`, the second demo
//! configuration's; or the `_` Storage uses; or a project number); every email is at `example.com`, or is a `dev-project` service
//! account in one of its spellings; every IPv4 address is private, from the
//! documentation ranges, the whole internet (`0.0.0.0/0`), or one of the
//! ranges Google documents for its own health checks, IAP and APIs. A value
//! cut short on screen (`…`) is held to being a prefix of an allowed one.

const PROJECT: &str = "dev-project";
const ALLOWED_DOMAINS: [&str; 4] = [
    "example.com",
    "dev-project.iam.gserviceaccount.com",
    "dev-project.iam",
    "developer.gserviceaccount.com",
];
const GOOGLE_RANGES: [[u8; 2]; 4] = [[35, 235], [130, 211], [35, 191], [199, 36]];

/// Every name in `text` that is not a fixture's, described.
pub(crate) fn problems(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices("projects/") {
        let project: String = text[at + "projects/".len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let ok = project.is_empty()
            || project == "_"
            || project.chars().all(|c| c.is_ascii_digit())
            || PROJECT.starts_with(&project)
            || project == "prod-project";
        if !ok {
            found.push(format!("project {project:?}"));
        }
    }
    // A project named outside a resource path: a header or preview's
    // `Project:` field, and a document's `projectId` / `project_id`.
    for marker in [
        "Project: ",
        "\"projectId\":\"",
        "\"projectId\": \"",
        "project_id\":\"",
        "project_id\": \"",
    ] {
        for (at, _) in text.match_indices(marker) {
            let project: String = text[at + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !project.is_empty() && !PROJECT.starts_with(&project) && project != "prod-project" {
                found.push(format!("project {project:?}"));
            }
        }
    }
    for word in text.split(|c: char| c.is_whitespace() || "\"',()[]{}<>│".contains(c)) {
        let word = word.trim_end_matches('…');
        if let Some((_, domain)) = word.split_once('@') {
            let domain = domain.trim_end_matches(['.', ':']);
            if !ALLOWED_DOMAINS.iter().any(|allowed| {
                *allowed == domain || (!domain.is_empty() && allowed.starts_with(domain))
            }) {
                found.push(format!("an address at {domain}"));
            }
        }
        let address = word.split('/').next().unwrap_or("");
        let parts: Vec<&str> = address.split('.').collect();
        let octets: Vec<u8> = parts.iter().filter_map(|o| o.parse().ok()).collect();
        if parts.len() == 4 && octets.len() == 4 {
            let ok = matches!(octets[..], [10, ..] | [192, 168, ..] | [0, 0, 0, 0])
                || (octets[0] == 172 && (16..32).contains(&octets[1]))
                || matches!(octets[..3], [192, 0, 2] | [198, 51, 100] | [203, 0, 113])
                || GOOGLE_RANGES.contains(&[octets[0], octets[1]]);
            if !ok {
                found.push(format!("the address {address}"));
            }
        }
    }
    found
}
