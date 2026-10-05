use crate::{
    Error, Result,
    models::AdoIdentity,
    organization::Organization,
    platform::{home_directory, localized_standard_cmp},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub organization: Organization,
    pub identity: AdoIdentity,
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    pub directory: PathBuf,
    profiles_path: PathBuf,
}
impl ProfileStore {
    pub fn new(directory: Option<PathBuf>) -> Result<Self> {
        let directory = match directory {
            Some(v) => v,
            None => home_directory()?.join(".config/ado"),
        };
        prepare_directory(&directory)?;
        Ok(Self {
            profiles_path: directory.join("profiles.json"),
            directory,
        })
    }
    pub fn load(&self) -> Result<Vec<Profile>> {
        if !self.profiles_path.exists() {
            return Ok(vec![]);
        }
        let meta = fs::symlink_metadata(&self.profiles_path)
            .map_err(|_| Error("Could not read the local profile store.".into()))?;
        if !meta.file_type().is_file()
            || meta.uid() != unsafe { libc::getuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(Error(
                "The profile file is invalid or contains duplicate profiles.".into(),
            ));
        }
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut file = options
            .open(&self.profiles_path)
            .map_err(|_| Error("Could not read the local profile store.".into()))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|_| Error("Could not read the local profile store.".into()))?;
        let mut profiles: Vec<Profile> = serde_json::from_slice(&data).map_err(|_| {
            Error("The profile file is invalid or contains duplicate profiles.".into())
        })?;
        validate_profiles(&profiles)?;
        profiles.sort_by(|a, b| localized_standard_cmp(&a.name, &b.name));
        Ok(profiles)
    }
    pub fn save(&self, profile: &Profile) -> Result<()> {
        validate_name(&profile.name)?;
        if profile.identity.id.trim().is_empty() {
            return Err(Error(
                "The profile file is invalid or contains duplicate profiles.".into(),
            ));
        }
        let mut profiles = self.load()?;
        if let Some(p) = profiles.iter().find(|p| {
            p.organization
                .name
                .eq_ignore_ascii_case(&profile.organization.name)
                && p.name != profile.name
        }) {
            return Err(Error(format!(
                "A profile already exists for Azure DevOps organization '{}'.",
                p.organization.name
            )));
        }
        if let Some(i) = profiles.iter().position(|p| p.name == profile.name) {
            profiles[i] = profile.clone()
        } else {
            profiles.push(profile.clone())
        }
        validate_profiles(&profiles)?;
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        self.write(&profiles)
    }
    pub fn remove(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let mut p = self.load()?;
        let Some(i) = p.iter().position(|x| x.name == name) else {
            return Err(Error(format!("Profile '{name}' was not found.")));
        };
        p.remove(i);
        self.write(&p)
    }
    pub fn profile_by_name(&self, name: &str) -> Result<Profile> {
        validate_name(name)?;
        self.load()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| Error(format!("Profile '{name}' was not found.")))
    }
    pub fn profile_by_organization(&self, org: &Organization) -> Result<Profile> {
        self.load()?.into_iter().find(|p|p.organization.name.eq_ignore_ascii_case(&org.name)).ok_or_else(||Error(format!("No profile is configured for '{}'. Run 'ado auth add NAME --org https://dev.azure.com/{}' in an interactive terminal.",org.name,org.name)))
    }
    fn write(&self, profiles: &[Profile]) -> Result<()> {
        let mut data = serde_json::to_vec_pretty(profiles)
            .map_err(|_| Error("Could not encode the local profile store.".into()))?;
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .directory
            .join(format!(".profiles-{}-{n}.tmp", std::process::id()));
        let mut o = fs::OpenOptions::new();
        o.write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut f = o
            .open(&temp)
            .map_err(|_| Error("Could not write the local profile store.".into()))?;
        let result = (|| {
            f.write_all(&data)
                .map_err(|_| Error("Could not write the local profile store.".into()))?;
            f.sync_all()
                .map_err(|_| Error("Could not write the local profile store.".into()))?;
            f.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Error("Could not write the local profile store.".into()))?;
            fs::rename(&temp, &self.profiles_path)
                .map_err(|_| Error("Could not replace the local profile store.".into()))?;
            Ok(())
        })();
        data.fill(0);
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if valid {
        Ok(())
    } else {
        Err(Error(format!(
            "Invalid profile name '{name}'. Use 1-64 letters, numbers, dots, underscores, or hyphens, starting with a letter or number."
        )))
    }
}
fn validate_profiles(profiles: &[Profile]) -> Result<()> {
    let mut names = HashSet::new();
    let mut orgs = HashSet::new();
    for p in profiles {
        validate_name(&p.name)?;
        if Organization::parse(&p.organization.name).ok().as_ref() != Some(&p.organization)
            || p.identity.id.trim().is_empty()
            || !names.insert(p.name.clone())
            || !orgs.insert(p.organization.name.to_ascii_lowercase())
        {
            return Err(Error(
                "The profile file is invalid or contains duplicate profiles.".into(),
            ));
        }
    }
    Ok(())
}
fn prepare_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|_| Error("Could not create the local profile store.".into()))?;
    let m = fs::symlink_metadata(path).map_err(|_| {
        Error(format!(
            "The profile directory is not a safe directory: {}",
            path.display()
        ))
    })?;
    if !m.file_type().is_dir() || m.uid() != unsafe { libc::getuid() } {
        return Err(Error(format!(
            "The profile directory is not a safe directory: {}",
            path.display()
        )));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| Error("Could not secure the local profile store.".into()))
}
