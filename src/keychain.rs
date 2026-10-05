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
        let organization = profile.organization.name.to_ascii_lowercase();
        format!(
            "{}:{}{}:{}",
            organization.len(),
            organization,
            profile.identity.id.len(),
            profile.identity.id
        )
    }

    fn entry(profile: &Profile) -> keyring::Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, &Self::account(profile))
    }
}

impl CredentialStore for KeychainStore {
    fn read(&self, profile: &Profile) -> Result<String> {
        let value = Self::entry(profile)
            .map_err(|error| operation_error("read", error))?
            .get_password()
            .map_err(|error| match error {
                keyring::Error::NoEntry => Error("No Keychain token was found for this profile. Run 'ado auth update NAME' for the affected profile.".into()),
                keyring::Error::BadEncoding(_) => invalid_token_data(),
                other => operation_error("read", other),
            })?;
        if value.is_empty() {
            Err(invalid_token_data())
        } else {
            Ok(value)
        }
    }

    fn save(&self, token: &str, profile: &Profile) -> Result<()> {
        if token.is_empty() {
            return Err(invalid_token_data());
        }
        Self::entry(profile)
            .map_err(|error| operation_error("save", error))?
            .set_password(token)
            .map_err(|error| operation_error("save", error))
    }

    fn remove(&self, profile: &Profile) -> Result<()> {
        match Self::entry(profile)
            .map_err(|error| operation_error("remove", error))?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(operation_error("remove", error)),
        }
    }
}

fn invalid_token_data() -> Error {
    Error("The token stored in Keychain is not valid text. Run 'ado auth update NAME' for the affected profile.".into())
}

fn operation_error(operation: &str, error: keyring::Error) -> Error {
    #[cfg(target_os = "linux")]
    {
        Error(format!(
            "Secret Service could not {operation} the token ({error}). Ensure a Secret Service provider such as gnome-keyring is installed, running, and unlocked."
        ))
    }
    #[cfg(target_os = "macos")]
    {
        use std::error::Error as _;
        let status = error
            .source()
            .and_then(|source| source.downcast_ref::<security_framework::base::Error>())
            .map(|source| (*source).code());
        match status {
            Some(status) => Error(format!(
                "Keychain could not {operation} the token (status {status})."
            )),
            None => Error(format!(
                "Keychain could not {operation} the token ({error})."
            )),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Error(format!(
            "The system credential store could not {operation} the token ({error})."
        ))
    }
}
