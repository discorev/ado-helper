mod common;

use common::*;

#[test]
fn profile_store_round_trips_without_token_and_uses_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("store"))).unwrap();
    let p = profile("work", "example", "user-1");
    s.save(&p).unwrap();
    assert_eq!(s.load().unwrap(), vec![p.clone()]);
    assert_eq!(s.profile_by_name("work").unwrap(), p);
    assert_eq!(
        s.profile_by_organization(&Organization::parse("example").unwrap())
            .unwrap(),
        p
    );
    let text = fs::read_to_string(s.directory.join("profiles.json")).unwrap();
    assert!(!text.contains("secret-token"));
    assert_eq!(
        fs::metadata(&s.directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(s.directory.join("profiles.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    )
}
#[test]
fn profile_store_rejects_duplicate_organization() {
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    s.save(&profile("first", "example", "1")).unwrap();
    assert_eq!(
        s.save(&profile("second", "example", "2"))
            .unwrap_err()
            .to_string(),
        "A profile already exists for Azure DevOps organization 'example'."
    )
}
#[test]
fn profile_store_validates_names_before_using_them() {
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    for n in ["", ".hidden", "../escape", "has space", &"a".repeat(65)] {
        assert_eq!(
            s.save(&profile(n, "example", "1")).unwrap_err().to_string(),
            format!(
                "Invalid profile name '{n}'. Use 1-64 letters, numbers, dots, underscores, or hyphens, starting with a letter or number."
            )
        )
    }
}
#[test]
fn profile_store_rejects_organization_bypassing_initializer() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    let p = s.directory.join("profiles.json");
    fs::write(&p,r#"[{"identity":{"displayName":"Test","id":"user-1","uniqueName":"test@example.com"},"name":"work","organization":{"name":"../not-an-org"}}]"#).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(s.load().is_err())
}
#[test]
fn keychain_account_separates_organization_and_identity() {
    let a = profile("one", "alpha", "same-id");
    let b = profile("two", "beta", "same-id");
    let c = profile("three", "alpha", "other-id");
    assert_ne!(KeychainStore::account(&a), KeychainStore::account(&b));
    assert_ne!(KeychainStore::account(&a), KeychainStore::account(&c));
    assert!(KeychainStore::account(&a).contains("alpha"));
    assert!(KeychainStore::account(&a).contains("same-id"))
}
