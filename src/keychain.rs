use crate::{Error, Result, profiles::Profile};

pub const SERVICE: &str = "dev.ollies.ado-helper.pat";
pub trait CredentialStore {
    fn read(&self, profile: &Profile) -> Result<String>;
    fn save(&self, token: &str, profile: &Profile) -> Result<()>;
    fn remove(&self, profile: &Profile) -> Result<()>;
}
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainStore;
impl KeychainStore {
    pub fn account(profile: &Profile) -> String {
        let o = profile.organization.name.to_ascii_lowercase();
        format!(
            "{}:{}{}:{}",
            o.len(),
            o,
            profile.identity.id.len(),
            profile.identity.id
        )
    }
    fn entry(profile: &Profile) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, &Self::account(profile)).map_err(map_backend)
    }
}
impl CredentialStore for KeychainStore {
    fn read(&self, profile: &Profile) -> Result<String> {
        let value=Self::entry(profile)?.get_password().map_err(|e|match e {keyring::Error::NoEntry=>Error("No Keychain token was found for this profile. Run 'ado auth update NAME' for the affected profile.".into()),other=>map_backend(other)})?;
        if value.is_empty() {
            Err(Error("The token stored in Keychain is not valid text. Run 'ado auth update NAME' for the affected profile.".into()))
        } else {
            Ok(value)
        }
    }
    fn save(&self, token: &str, profile: &Profile) -> Result<()> {
        if token.is_empty() {
            return Err(Error("The token stored in Keychain is not valid text. Run 'ado auth update NAME' for the affected profile.".into()));
        }
        Self::entry(profile)?
            .set_password(token)
            .map_err(map_backend)
    }
    fn remove(&self, profile: &Profile) -> Result<()> {
        match Self::entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_backend(e)),
        }
    }
}
fn map_backend(e: keyring::Error) -> Error {
    #[cfg(target_os = "linux")]
    {
        Error(format!(
            "Secret Service could not access the token ({e}). Ensure a Secret Service provider such as gnome-keyring is installed, running, and unlocked."
        ))
    }
    #[cfg(target_os = "macos")]
    {
        Error(format!("Keychain could not access the token ({e})."))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Error(format!(
            "The system credential store could not access the token ({e})."
        ))
    }
}
