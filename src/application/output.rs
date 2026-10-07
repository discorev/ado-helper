use ado_core::{Error, Result};
use serde_json::Value;
use std::{fs, path::PathBuf};

pub(super) fn standardize_path(value: String) -> Result<PathBuf> {
    use std::path::Component;
    let path = PathBuf::from(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

pub(super) fn read_body(path: &str) -> Result<String> {
    let m =
        fs::metadata(path).map_err(|_| Error("Could not read the comment body file.".into()))?;
    if m.len() > 1_048_576 {
        return Err(Error("Comment body file must be 1 MiB or smaller.".into()));
    }
    let data = fs::read(path).map_err(|_| Error("Could not read the comment body file.".into()))?;
    let body = String::from_utf8(data)
        .map_err(|_| Error("Comment body file must contain nonempty UTF-8 text.".into()))?;
    if body.trim().is_empty() {
        Err(Error(
            "Comment body file must contain nonempty UTF-8 text.".into(),
        ))
    } else {
        Ok(body)
    }
}
pub(super) fn integer(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|x| i64::try_from(x).ok()))
}
pub(super) fn write_json(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
pub(super) fn write_value<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
