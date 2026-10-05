use crate::{Error, Result};
use std::{cmp::Ordering, ffi::CStr, path::PathBuf};
use unicode_general_category::{GeneralCategory, get_general_category};

pub fn is_swift_control(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::Control | GeneralCategory::Format
    )
}

pub fn terminal_safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if is_swift_control(c) { '�' } else { c })
        .collect()
}

pub fn home_directory() -> Result<PathBuf> {
    #[cfg(not(target_os = "macos"))]
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home));
    }

    passwd_home().ok_or_else(|| Error("Could not determine the home directory.".into()))
}

fn passwd_home() -> Option<PathBuf> {
    let uid = unsafe { libc::getuid() };
    let size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer = vec![0_u8; if size > 0 { size as usize } else { 16_384 }];
    let mut entry = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
    if bytes.is_empty() {
        None
    } else {
        use std::os::unix::ffi::OsStrExt;
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
}

pub fn localized_standard_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.as_bytes().iter().copied().peekable();
    let mut right = b.as_bytes().iter().copied().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut l = Vec::new();
                let mut r = Vec::new();
                while left.peek().is_some_and(u8::is_ascii_digit) {
                    l.push(left.next().unwrap());
                }
                while right.peek().is_some_and(u8::is_ascii_digit) {
                    r.push(right.next().unwrap());
                }
                let ln = l.iter().position(|c| *c != b'0').unwrap_or(l.len());
                let rn = r.iter().position(|c| *c != b'0').unwrap_or(r.len());
                let ord = l[ln..]
                    .len()
                    .cmp(&r[rn..].len())
                    .then_with(|| l[ln..].cmp(&r[rn..]))
                    .then_with(|| l.len().cmp(&r.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(_), Some(_)) => {
                let x = left.next().unwrap().to_ascii_lowercase();
                let y = right.next().unwrap().to_ascii_lowercase();
                let ord = x.cmp(&y);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_home_ignores_home_environment_override() {
        let expected = home_directory().unwrap();
        let previous = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", "/tmp/ado-fake-home") };
        let actual = home_directory().unwrap();
        match previous {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(actual, expected);
    }
}
