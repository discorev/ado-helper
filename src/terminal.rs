use crate::{Error, Result};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    mem::MaybeUninit,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    sync::atomic::{AtomicI32, Ordering},
};

static TERMINAL_FD: AtomicI32 = AtomicI32::new(-1);
static mut ORIGINAL_TERMIOS: MaybeUninit<libc::termios> = MaybeUninit::uninit();
static mut HIDDEN_TERMIOS: MaybeUninit<libc::termios> = MaybeUninit::uninit();
const RESTORED_SIGNALS: [libc::c_int; 5] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGTSTP,
];

extern "C" fn restore_terminal_for_signal(signal: libc::c_int) {
    let fd = TERMINAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let original = unsafe { std::ptr::read(std::ptr::addr_of!(ORIGINAL_TERMIOS).cast()) };
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &original) };
    }
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
    if signal == libc::SIGTSTP && fd >= 0 {
        let hidden = unsafe { std::ptr::read(std::ptr::addr_of!(HIDDEN_TERMIOS).cast()) };
        unsafe {
            libc::tcsetattr(fd, libc::TCSAFLUSH, &hidden);
            libc::signal(
                libc::SIGTSTP,
                restore_terminal_for_signal as libc::sighandler_t,
            );
        }
    }
}

struct EchoGuard {
    fd: libc::c_int,
    original: libc::termios,
    handlers: [(libc::c_int, libc::sighandler_t); RESTORED_SIGNALS.len()],
}
impl EchoGuard {
    fn hide(fd: libc::c_int, original: libc::termios) -> Result<Self> {
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        unsafe {
            std::ptr::write(std::ptr::addr_of_mut!(ORIGINAL_TERMIOS).cast(), original);
            std::ptr::write(std::ptr::addr_of_mut!(HIDDEN_TERMIOS).cast(), hidden);
        }
        TERMINAL_FD.store(fd, Ordering::SeqCst);
        let mut handlers = [(0, libc::SIG_DFL); RESTORED_SIGNALS.len()];
        for (slot, signal) in handlers.iter_mut().zip(RESTORED_SIGNALS) {
            let previous =
                unsafe { libc::signal(signal, restore_terminal_for_signal as libc::sighandler_t) };
            *slot = (signal, previous);
        }
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &hidden) } != 0 {
            for (signal, previous) in handlers {
                unsafe { libc::signal(signal, previous) };
            }
            TERMINAL_FD.store(-1, Ordering::SeqCst);
            return Err(read_error());
        }
        Ok(Self {
            fd,
            original,
            handlers,
        })
    }
}
impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original) };
        TERMINAL_FD.store(-1, Ordering::SeqCst);
        for (signal, previous) in self.handlers {
            unsafe { libc::signal(signal, previous) };
        }
    }
}

fn tty() -> Result<std::fs::File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/tty")
        .map_err(|_| no_terminal())?;
    if unsafe { libc::isatty(f.as_raw_fd()) } != 1 {
        Err(no_terminal())
    } else {
        Ok(f)
    }
}
pub fn require_interactive() -> Result<()> {
    tty().map(drop)
}
pub fn read_secret(prompt: &str) -> Result<String> {
    let mut f = tty()?;
    f.write_all(prompt.as_bytes()).map_err(|_| read_error())?;
    f.flush().ok();
    let fd = f.as_raw_fd();
    let mut old = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut old) } != 0 {
        return Err(read_error());
    }
    let guard = EchoGuard::hide(fd, old)?;
    let result = read_line(&mut f, 4096);
    drop(guard);
    let _ = f.write_all(b"\n");
    result
}
pub fn confirm(prompt: &str) -> Result<bool> {
    let mut f = tty()?;
    write!(f, "{prompt} [y/N] ").map_err(|_| read_error())?;
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
            Ok(_) => return Err(read_error()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(read_error()),
        }
    }
    String::from_utf8(bytes).map_err(|_| read_error())
}
fn no_terminal() -> Error {
    Error(
        "Authentication needs an interactive terminal. Run this command directly in a terminal."
            .into(),
    )
}
fn read_error() -> Error {
    Error("Could not read from the interactive terminal.".into())
}
