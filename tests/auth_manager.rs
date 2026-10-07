mod common;

use common::*;

#[derive(Default)]
struct AuthState {
    profiles: Vec<Profile>,
    saved: Vec<Profile>,
    tokens: Vec<(String, Profile)>,
    removed: Vec<Profile>,
    urls: Vec<String>,
    messages: Vec<String>,
    prompts: Vec<String>,
    validations: usize,
    interactive: usize,
    cleanup: usize,
    removals: usize,
    token: String,
    identity: Option<AdoIdentity>,
    confirm: bool,
    save_fail: bool,
    cleanup_fail: bool,
    interactive_fail: bool,
    remove_fail: bool,
    validation_fail: bool,
}
impl AuthOps for AuthState {
    fn load_profiles(&mut self) -> Result<Vec<Profile>> {
        Ok(self.profiles.clone())
    }
    fn save_profile(&mut self, p: &Profile) -> Result<()> {
        if self.save_fail {
            return Err(Error("fail".into()));
        }
        self.saved.push(p.clone());
        self.profiles.push(p.clone());
        Ok(())
    }
    fn remove_profile(&mut self, n: &str) -> Result<()> {
        self.removals += 1;
        if self.remove_fail {
            return Err(Error("fail".into()));
        }
        self.profiles.retain(|p| p.name != n);
        Ok(())
    }
    fn save_token(&mut self, t: &str, p: &Profile) -> Result<()> {
        self.tokens.push((t.into(), p.clone()));
        Ok(())
    }
    fn remove_token(&mut self, p: &Profile) -> Result<()> {
        self.cleanup += 1;
        if self.cleanup_fail {
            return Err(Error("fail".into()));
        }
        self.removed.push(p.clone());
        Ok(())
    }
    fn read_secret(&mut self, _: &str) -> Result<String> {
        Ok(self.token.clone())
    }
    fn confirm(&mut self, p: &str) -> Result<bool> {
        self.prompts.push(p.into());
        Ok(self.confirm)
    }
    fn require_interactive(&mut self) -> Result<()> {
        self.interactive += 1;
        if self.interactive_fail {
            Err(Error("Authentication needs an interactive terminal. Run this command directly in a terminal.".into()))
        } else {
            Ok(())
        }
    }
    fn open_url(&mut self, u: &str) -> Result<()> {
        self.urls.push(u.into());
        Ok(())
    }
    fn validate_identity(&mut self, _: &Organization, _: &str) -> Result<AdoIdentity> {
        self.validations += 1;
        if self.validation_fail {
            Err(Error("failure".into()))
        } else {
            Ok(self.identity.clone().unwrap())
        }
    }
    fn output(&mut self, m: &str) {
        self.messages.push(m.into())
    }
}
fn auth_state(token: &str, id: &str) -> AuthState {
    AuthState {
        token: token.into(),
        identity: Some(AdoIdentity {
            id: id.into(),
            display_name: "Example User".into(),
            unique_name: "user@example.com".into(),
        }),
        confirm: true,
        ..Default::default()
    }
}
#[test]
fn add_opens_pat_page_validates_and_saves_after_confirmation() {
    let mut s = auth_state(&"a".repeat(52), "identity-1");
    auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(
        s.urls,
        vec!["https://dev.azure.com/example/_usersSettings/tokens"]
    );
    assert_eq!(s.validations, 1);
    assert_eq!(
        s.tokens
            .iter()
            .map(|(token, _)| token.as_str())
            .collect::<Vec<_>>(),
        vec!["a".repeat(52)]
    );
    assert_eq!(
        s.saved
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["work"]
    );
    assert_eq!(s.tokens[0].1.identity, s.saved[0].identity);
    assert_eq!(s.prompts.len(), 1);
    assert!(s.prompts[0].contains("Example User"));
    assert!(s.prompts[0].contains("user@example.com"));
    assert!(s.messages.join("\n").contains("vso.threads_full"));
    assert!(!s.messages.join("\n").contains(&"a".repeat(52)))
}
#[test]
fn invalid_token_never_calls_network_or_storage() {
    let mut s = auth_state("short", "identity-1");
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "The token is not plausible. Paste the complete PAT without spaces or line breaks."
    );
    assert_eq!(s.validations, 0);
    assert!(s.tokens.is_empty());
    assert!(s.saved.is_empty());
    assert!(s.urls.is_empty())
}
#[test]
fn update_with_different_identity_leaves_old_token_untouched() {
    let mut s = auth_state(&"b".repeat(52), "identity-2");
    s.profiles.push(profile("work", "example", "identity-1"));
    assert_eq!(
        auth::update(&mut s, "work", false).unwrap_err().to_string(),
        "The new token belongs to a different Azure DevOps identity. The existing token was not changed."
    );
    assert!(s.tokens.is_empty());
    assert!(s.prompts.is_empty())
}
#[test]
fn validation_failure_leaves_old_token_untouched() {
    let mut s = auth_state(&"c".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.validation_fail = true;
    assert!(auth::update(&mut s, "work", false).is_err());
    assert!(s.tokens.is_empty());
    assert!(s.prompts.is_empty())
}
#[test]
fn add_rolls_back_keychain_when_profile_save_fails() {
    let mut s = auth_state(&"d".repeat(52), "identity-1");
    s.save_fail = true;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "Authentication succeeded, but the local profile could not be saved. No token was retained."
    );
    assert_eq!(s.tokens.len(), 1);
    assert_eq!(s.removed[0].name, "work")
}
#[test]
fn cancelled_confirmation_does_not_save() {
    let mut s = auth_state(&"e".repeat(52), "identity-1");
    s.confirm = false;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "Authentication was cancelled; no credentials were saved."
    );
    assert_eq!(s.validations, 1);
    assert!(s.tokens.is_empty());
    assert!(s.saved.is_empty())
}
#[test]
fn identity_confirmation_includes_names() {
    let mut s = auth_state(&"f".repeat(52), "identity-1");
    s.identity = Some(AdoIdentity {
        id: "identity-1".into(),
        display_name: "Alex Smith".into(),
        unique_name: "alex.one@example.com".into(),
    });
    auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        false,
    )
    .unwrap();
    assert!(s.prompts[0].contains("Alex Smith <alex.one@example.com>"))
}
#[test]
fn noninteractive_fails_before_browser() {
    let mut s = auth_state(&"g".repeat(52), "identity-1");
    s.interactive_fail = true;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            true
        )
        .unwrap_err()
        .to_string(),
        "Authentication needs an interactive terminal. Run this command directly in a terminal."
    );
    assert_eq!(s.interactive, 1);
    assert!(s.urls.is_empty());
    assert_eq!(s.validations, 0)
}
#[test]
fn profile_save_and_cleanup_failure_reports_retained_item() {
    let mut s = auth_state(&"h".repeat(52), "identity-1");
    s.save_fail = true;
    s.cleanup_fail = true;
    let e = auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        false,
    )
    .unwrap_err();
    assert!(e.to_string().contains("Keychain"));
    assert_eq!(s.cleanup, 1);
    assert_eq!(s.tokens.len(), 1)
}
#[test]
fn remove_keychain_failure_preserves_profile() {
    let mut s = auth_state(&"i".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.cleanup_fail = true;
    assert!(auth::remove(&mut s, "work").is_err());
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.cleanup, 1);
    assert_eq!(s.removals, 0)
}
#[test]
fn remove_profile_failure_reports_token_gone() {
    let mut s = auth_state(&"j".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.remove_fail = true;
    let e = auth::remove(&mut s, "work").unwrap_err();
    assert!(e.to_string().contains("Keychain token was removed"));
    assert!(e.to_string().contains("profile remains without a token"));
    assert_eq!(s.removed.len(), 1);
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.removals, 1)
}
#[test]
fn scope_guidance_uses_least_privilege() {
    let o = Organization::parse("example").unwrap();
    let g = auth::scope_instructions(&o);
    assert!(g.contains("vso.code"));
    assert!(g.contains("vso.threads_full"));
    assert!(g.contains("vso.build"));
    assert!(g.contains("Do not choose Full access"));
    assert!(!g.contains("vso.code_write"));
    assert!(g.contains("organization policy"));
    assert_eq!(
        auth::pat_settings_url(&o),
        "https://dev.azure.com/example/_usersSettings/tokens"
    )
}
