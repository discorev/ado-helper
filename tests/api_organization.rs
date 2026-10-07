mod common;

use common::*;

#[test]
fn organization_normalizes_supported_forms() {
    let b = Organization::parse("Example-Org").unwrap();
    assert_eq!(b.name, "example-org");
    assert_eq!(b.url(), "https://dev.azure.com/example-org");
    let modern = Organization::parse("https://dev.azure.com/example-org/").unwrap();
    assert_eq!(modern.name, "example-org");
    assert_eq!(modern.url(), "https://dev.azure.com/example-org");
    let legacy = Organization::parse("https://LegacyOrg.visualstudio.com/").unwrap();
    assert_eq!(legacy.name, "legacyorg");
    assert_eq!(legacy.url(), "https://dev.azure.com/legacyorg")
}
#[test]
fn organization_rejects_ambiguous_or_unsafe_inputs() {
    for v in [
        "http://dev.azure.com/org",
        "https://user:password@dev.azure.com/org",
        "https://dev.azure.com:443/org",
        "https://dev.azure.com/org/project",
        "https://dev.azure.com/org?token=secret",
        "https://dev.azure.com/org#fragment",
        "https://sub.org.visualstudio.com",
        "bad/name",
        "-bad",
        "bad-",
        " bad",
    ] {
        assert!(Organization::parse(v).is_err(), "{v}")
    }
}
#[test]
fn organization_codable_decoding_revalidates_and_normalizes() {
    let n: Organization = serde_json::from_str("{\"name\":\"FABRIKAM\"}").unwrap();
    assert_eq!(n.name, "fabrikam");
    assert!(
        serde_json::from_str::<Organization>("{\"name\":\"https://evil.example/org\"}").is_err()
    )
}
#[test]
fn pr_locator_parses_modern_and_legacy_urls() {
    let m = PrLocator::parse("https://dev.azure.com/acme/My%20Project/_git/Repo/pullrequest/42")
        .unwrap();
    assert_eq!(m.organization, Organization::parse("acme").unwrap());
    assert_eq!(m.project.as_deref(), Some("My Project"));
    assert_eq!(m.repository.as_deref(), Some("Repo"));
    assert_eq!(m.id, 42);
    let l =
        PrLocator::parse("https://acme.visualstudio.com/Project/_git/Repo/pullrequest/7/").unwrap();
    assert_eq!(l.organization, Organization::parse("acme").unwrap());
    assert_eq!(l.project.as_deref(), Some("Project"));
    assert_eq!(l.repository.as_deref(), Some("Repo"));
    assert_eq!(l.id, 7)
}
#[test]
fn pr_locator_rejects_non_pr_and_encoded_path_separator() {
    for v in [
        "http://dev.azure.com/acme/Project/_git/Repo/pullrequest/1",
        "https://evil.example/acme/Project/_git/Repo/pullrequest/1",
        "https://dev.azure.com/acme/Project/_git/Repo/pullrequest/0",
        "https://dev.azure.com/acme/Project%2FExtra/_git/Repo/pullrequest/1",
    ] {
        assert!(PrLocator::parse(v).is_err(), "{v}")
    }
}
