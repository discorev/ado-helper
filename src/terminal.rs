use crate::{Error, Result};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
};
fn tty() -> Result<std::fs::File> {
    let f=OpenOptions::new().read(true).write(true).custom_flags(libc::O_CLOEXEC).open("/dev/tty").map_err(|_|Error("Authentication needs an interactive terminal. Run this command directly in a terminal.".into()))?;
    if unsafe { libc::isatty(f.as_raw_fd()) } != 1 {
        Err(Error("Authentication needs an interactive terminal. Run this command directly in a terminal.".into()))
    } else {
        Ok(f)
    }
}
pub fn require_interactive() -> Result<()> {
    tty().map(drop)
}
pub fn read_secret(prompt: &str) -> Result<String> {
    let mut f = tty()?;
    f.write_all(prompt.as_bytes())
        .map_err(|_| Error("Could not read from the interactive terminal.".into()))?;
    f.flush().ok();
    let fd = f.as_raw_fd();
    let mut old = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut old) } != 0 {
        return Err(Error(
            "Could not read from the interactive terminal.".into(),
        ));
    }
    let mut hidden = old;
    hidden.c_lflag &= !(libc::ECHO);
    if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &hidden) } != 0 {
        return Err(Error(
            "Could not read from the interactive terminal.".into(),
        ));
    }
    let result = read_line(&mut f, 4096);
    unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &old) };
    let _ = f.write_all(b"\n");
    result
}
pub fn confirm(prompt: &str) -> Result<bool> {
    let mut f = tty()?;
    write!(f, "{prompt} [y/N] ")
        .map_err(|_| Error("Could not read from the interactive terminal.".into()))?;
    f.flush().ok();
    let v = read_line(&mut f, 31)?;
    Ok(v.trim().eq_ignore_ascii_case("y") || v.trim().eq_ignore_ascii_case("yes"))
}
fn read_line(f: &mut std::fs::File, max: usize) -> Result<String> {
    let mut bytes = Vec::new();
    let mut one = [0u8];
    loop {
        match f.read(&mut one) {
            Ok(1) if one[0] == b'\n' || one[0] == b'\r' => break,
            Ok(1) => {
                if bytes.len() >= max {
                    return Err(Error("The entered token is unexpectedly long.".into()));
                }
                bytes.push(one[0])
            }
            Ok(_) => {
                return Err(Error(
                    "Could not read from the interactive terminal.".into(),
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                return Err(Error(
                    "Could not read from the interactive terminal.".into(),
                ));
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| Error("Could not read from the interactive terminal.".into()))
}
