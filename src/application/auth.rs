use super::Application;
use ado_core::{
    Error, Result,
    ado_client::AdoClient,
    auth::{self, ProductionAuth},
    keychain::CredentialStore,
    organization::Organization,
    platform::terminal_safe,
};

impl Application {
    pub(super) fn auth_add(
        &self,
        name: &str,
        organization: &str,
        open_browser: bool,
    ) -> Result<()> {
        let mut auth = ProductionAuth {
            store: &self.store,
            keychain: &self.keychain,
        };
        auth::add(
            &mut auth,
            name,
            Organization::parse(organization)?,
            open_browser,
        )
    }

    pub(super) fn auth_update(&self, name: &str, open_browser: bool) -> Result<()> {
        let mut auth = ProductionAuth {
            store: &self.store,
            keychain: &self.keychain,
        };
        auth::update(&mut auth, name, open_browser)
    }

    pub(super) fn auth_status(&self, check: bool) -> Result<()> {
        let profiles = self.store.load()?;
        if profiles.is_empty() {
            println!("No authentication profiles configured.");
            return Ok(());
        }
        for p in profiles {
            if check {
                let token = self.keychain.read(&p)?;
                let id = AdoClient::new(p.organization.clone(), &token)?.identity()?;
                if id.id != p.identity.id {
                    return Err(Error(format!(
                        "Profile '{}' authenticated as a different identity.",
                        p.name
                    )));
                }
                println!(
                    "{}\t{}\t{}\tverified",
                    p.name,
                    p.organization.url(),
                    terminal_safe(&id.unique_name)
                );
            } else {
                println!(
                    "{}\t{}\t{}",
                    p.name,
                    p.organization.url(),
                    terminal_safe(&p.identity.unique_name)
                );
            }
        }
        Ok(())
    }

    pub(super) fn auth_remove(&self, name: &str) -> Result<()> {
        let mut auth = ProductionAuth {
            store: &self.store,
            keychain: &self.keychain,
        };
        auth::remove(&mut auth, name)
    }
}
