use crate::{
    Error, Result,
    ado_client::AdoClient,
    keychain::{CredentialStore, KeychainStore},
    models::AdoIdentity,
    organization::Organization,
    profiles::{Profile, ProfileStore, validate_name},
    terminal,
};
use std::process::Command;

pub trait AuthOps {
    fn load_profiles(&mut self) -> Result<Vec<Profile>>;
    fn save_profile(&mut self, p: &Profile) -> Result<()>;
    fn remove_profile(&mut self, n: &str) -> Result<()>;
    fn save_token(&mut self, t: &str, p: &Profile) -> Result<()>;
    fn remove_token(&mut self, p: &Profile) -> Result<()>;
    fn read_secret(&mut self, prompt: &str) -> Result<String>;
    fn confirm(&mut self, prompt: &str) -> Result<bool>;
    fn require_interactive(&mut self) -> Result<()>;
    fn open_url(&mut self, url: &str) -> Result<()>;
    fn validate_identity(&mut self, org: &Organization, token: &str) -> Result<AdoIdentity>;
    fn output(&mut self, message: &str);
}
pub struct ProductionAuth<'a> {
    pub store: &'a ProfileStore,
    pub keychain: &'a KeychainStore,
}
impl AuthOps for ProductionAuth<'_> {
    fn load_profiles(&mut self) -> Result<Vec<Profile>> {
        self.store.load()
    }
    fn save_profile(&mut self, p: &Profile) -> Result<()> {
        self.store.save(p)
    }
    fn remove_profile(&mut self, n: &str) -> Result<()> {
        self.store.remove(n)
    }
    fn save_token(&mut self, t: &str, p: &Profile) -> Result<()> {
        self.keychain.save(t, p)
    }
    fn remove_token(&mut self, p: &Profile) -> Result<()> {
        self.keychain.remove(p)
    }
    fn read_secret(&mut self, p: &str) -> Result<String> {
        terminal::read_secret(p)
    }
    fn confirm(&mut self, p: &str) -> Result<bool> {
        terminal::confirm(p)
    }
    fn require_interactive(&mut self) -> Result<()> {
        terminal::require_interactive()
    }
    fn open_url(&mut self, u: &str) -> Result<()> {
        open_url(u)
    }
    fn validate_identity(&mut self, o: &Organization, t: &str) -> Result<AdoIdentity> {
        AdoClient::new(o.clone(), t)?.identity()
    }
    fn output(&mut self, m: &str) {
        eprintln!("{m}")
    }
}

pub fn add<O: AuthOps>(
    ops: &mut O,
    name: &str,
    organization: Organization,
    open_browser: bool,
) -> Result<()> {
    validate_name(name)?;
    let ps = ops.load_profiles()?;
    if ps.iter().any(|p| p.name == name) {
        return Err(Error(format!(
            "A profile named '{name}' already exists. Use 'ado auth update {name}' to rotate its token."
        )));
    }
    if ps
        .iter()
        .any(|p| p.organization.name.eq_ignore_ascii_case(&organization.name))
    {
        return Err(Error(format!(
            "Azure DevOps organization '{}' already has a profile. Update that profile instead.",
            organization.name
        )));
    }
    prepare(ops, &organization, open_browser)?;
    let mut token = ops.read_secret("Azure DevOps PAT: ")?;
    let result = (|| {
        validate_token(&token)?;
        let identity = ops.validate_identity(&organization, &token)?;
        if identity.id.trim().is_empty() {
            return Err(Error("Azure DevOps did not return a usable authenticated identity. No credentials were saved.".into()));
        }
        let profile = Profile {
            name: name.into(),
            organization: organization.clone(),
            identity: identity.clone(),
        };
        let desc = identity_description(&identity);
        if !ops.confirm(&format!(
            "Authenticated as {desc} for {}. Save this profile?",
            organization.name
        ))? {
            return Err(Error(
                "Authentication was cancelled; no credentials were saved.".into(),
            ));
        }
        ops.save_token(&token, &profile)?;
        if ops.save_profile(&profile).is_err() {
            if ops.remove_token(&profile).is_err() {
                return Err(Error("The local profile could not be saved, and the new Keychain item could not be removed. No profile was created; remove the ado token in Keychain Access before retrying.".into()));
            }
            return Err(Error("Authentication succeeded, but the local profile could not be saved. No token was retained.".into()));
        }
        ops.output(&format!(
            "Saved profile '{name}' for {} as {desc}.",
            organization.url()
        ));
        Ok(())
    })();
    unsafe { token.as_bytes_mut().fill(0) };
    result
}
pub fn update<O: AuthOps>(ops: &mut O, name: &str, open_browser: bool) -> Result<()> {
    let profile = profile_named(ops, name)?;
    prepare(ops, &profile.organization, open_browser)?;
    let mut token = ops.read_secret("New Azure DevOps PAT: ")?;
    let result = (|| {
        validate_token(&token)?;
        let id = ops.validate_identity(&profile.organization, &token)?;
        if id.id.trim().is_empty() {
            return Err(Error("Azure DevOps did not return a usable authenticated identity. No credentials were saved.".into()));
        }
        if id.id != profile.identity.id {
            return Err(Error("The new token belongs to a different Azure DevOps identity. The existing token was not changed.".into()));
        }
        let desc = identity_description(&id);
        if !ops.confirm(&format!(
            "Validated {desc} for {}. Replace the stored token?",
            profile.organization.name
        ))? {
            return Err(Error(
                "Authentication was cancelled; no credentials were saved.".into(),
            ));
        }
        ops.save_token(&token, &profile)?;
        ops.output(&format!("Updated the Keychain token for profile '{name}'."));
        Ok(())
    })();
    unsafe { token.as_bytes_mut().fill(0) };
    result
}
pub fn remove<O: AuthOps>(ops: &mut O, name: &str) -> Result<()> {
    let p = profile_named(ops, name)?;
    ops.remove_token(&p)?;
    if ops.remove_profile(name).is_err() {
        return Err(Error(format!(
            "The Keychain token was removed, but local profile '{name}' could not be removed. The profile remains without a token; retry 'ado auth remove {name}'."
        )));
    }
    ops.output(&format!("Removed local profile '{name}' and its Keychain token. The PAT was not revoked in Azure DevOps."));
    Ok(())
}
fn profile_named<O: AuthOps>(ops: &mut O, name: &str) -> Result<Profile> {
    validate_name(name)?;
    ops.load_profiles()?
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| Error(format!("Profile '{name}' was not found.")))
}
fn prepare<O: AuthOps>(ops: &mut O, org: &Organization, browser: bool) -> Result<()> {
    ops.require_interactive()?;
    ops.output(&scope_instructions(org));
    if browser && ops.open_url(&pat_settings_url(org)).is_err() {
        return Err(Error("Could not open the Azure DevOps token settings page. Open the displayed URL in a browser and try again with --no-browser.".into()));
    }
    Ok(())
}
pub fn validate_token(token: &str) -> Result<()> {
    if !(20..=2048).contains(&token.len())
        || token.trim() != token
        || token.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        Err(Error(
            "The token is not plausible. Paste the complete PAT without spaces or line breaks."
                .into(),
        ))
    } else {
        Ok(())
    }
}
pub fn pat_settings_url(org: &Organization) -> String {
    format!("{}/_usersSettings/tokens", org.url())
}
pub fn scope_instructions(org: &Organization) -> String {
    format!(
        "Create an organization-scoped, short-lived PAT for {}:\n  Required: Code — Read (vso.code)\n  Required: PR threads — Read & write (vso.threads_full; use \"Show all scopes\")\n  Not needed by current ado commands: Build — Read (vso.build)\nDo not choose Full access or Code read/write. If \"PR threads\" is unavailable,\ncheck your organization policy with an administrator rather than broadening the token.\nToken settings: {}",
        org.name,
        pat_settings_url(org)
    )
}
fn identity_description(i: &AdoIdentity) -> String {
    let d = safe(&i.display_name);
    let u = safe(&i.unique_name);
    if d.is_empty() {
        u
    } else if u.is_empty() {
        d
    } else {
        format!("{d} <{u}>")
    }
}
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let executable = "/usr/bin/open";
    #[cfg(target_os = "linux")]
    let executable = "xdg-open";
    let status = Command::new(executable)
        .arg(url)
        .status()
        .map_err(|_| Error("browser".into()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error("browser".into()))
    }
}
