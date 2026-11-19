//! What a demo document or a demo screen may name.
//!
//! Every project is `dev-project` (or `prod-project`, the second demo
//! configuration's; or the `_` Storage uses; or a project number); every email is at `example.com`, or is a `dev-project` service
//! account in one of its spellings; every IPv4 address is private, from the
//! documentation ranges, the whole internet (`0.0.0.0/0`), or inside one of
//! the ranges Google documents for its own health checks, IAP and Private
//! Google Access, matched on the whole range. A value cut short on screen
//! (`…` or `...` straight after it) is held to being a prefix of an allowed one; a
//! value that was not cut is held to being one.

const PROJECT: &str = "dev-project";
const ALLOWED_DOMAINS: [&str; 4] = [
    "example.com",
    "dev-project.iam.gserviceaccount.com",
    "dev-project.iam",
    "developer.gserviceaccount.com",
];
/// Google's documented ranges, as (network, prefix length): IAP TCP
/// forwarding, the two health-check ranges, and Private Google Access.
const GOOGLE_RANGES: [([u8; 4], u32); 4] = [
    ([35, 235, 240, 0], 20),
    ([130, 211, 0, 0], 22),
    ([35, 191, 0, 0], 16),
    ([199, 36, 153, 8], 30),
];

/// Whether `address` lies inside one of [`GOOGLE_RANGES`].
fn in_google_range(address: [u8; 4]) -> bool {
    let value = u32::from_be_bytes(address);
    GOOGLE_RANGES.iter().any(|(network, length)| {
        let mask = u32::MAX << (32 - length);
        value & mask == u32::from_be_bytes(*network) & mask
    })
}

/// Whether a project read off `text` at `end` is one the demo may name. A
/// prefix of one is allowed only where the screen cut it: `…` follows.
fn allowed_project(project: &str, text: &str, end: usize) -> bool {
    let cut = text[end..].starts_with('…') || text[end..].starts_with("...");
    project == PROJECT
        || project == "prod-project"
        || (cut && (PROJECT.starts_with(project) || "prod-project".starts_with(project)))
}

/// Every name in `text` that is not a fixture's, described.
pub(crate) fn problems(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices("projects/") {
        let start = at + "projects/".len();
        let project: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let ok = project.is_empty()
            || project == "_"
            || project.chars().all(|c| c.is_ascii_digit())
            || allowed_project(&project, text, start + project.len());
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
            let start = at + marker.len();
            let project: String = text[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !project.is_empty() && !allowed_project(&project, text, start + project.len()) {
                found.push(format!("project {project:?}"));
            }
        }
    }
    for word in text.split(|c: char| c.is_whitespace() || "\"',()[]{}<>│".contains(c)) {
        let cut = word.ends_with('…') || word.ends_with("...");
        let word = word.trim_end_matches('…');
        if let Some((_, domain)) = word.split_once('@') {
            let domain = domain.trim_end_matches(['.', ':']);
            if !ALLOWED_DOMAINS.iter().any(|allowed| {
                *allowed == domain || (cut && !domain.is_empty() && allowed.starts_with(domain))
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
                || in_google_range([octets[0], octets[1], octets[2], octets[3]]);
            if !ok {
                found.push(format!("the address {address}"));
            }
        }
    }
    found
}

#[test]
fn a_prefix_passes_only_where_the_screen_cut_it() {
    assert!(problems("ci@dev-project.iam.gservi…").is_empty());
    assert!(problems("ci@dev-project.iam.gservi...").is_empty());
    assert!(!problems("ci@dev-project.iam.gservi").is_empty());
    assert!(problems("projects/dev-pro…").is_empty());
    assert!(!problems("projects/dev-pro/x").is_empty());
    assert!(!problems("Project: dev").is_empty());
}

#[test]
fn google_ranges_match_on_the_whole_range() {
    for inside in [
        "35.235.240.1",
        "35.235.255.254",
        "130.211.3.9",
        "35.191.7.7",
        "199.36.153.11",
    ] {
        assert!(problems(inside).is_empty(), "{inside} is Google's");
    }
    for outside in [
        "35.235.239.1",
        "130.211.4.1",
        "199.36.153.12",
        "199.36.153.7",
        "35.192.0.1",
    ] {
        assert!(!problems(outside).is_empty(), "{outside} is not Google's");
    }
}
