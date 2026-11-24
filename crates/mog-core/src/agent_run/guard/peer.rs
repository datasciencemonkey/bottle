//! Who is on the other end of the app socket (Section 2.3, A4): the uid of the peer, its PID and
//! the PID of its parent. The peer is the shim. Its parent must be the `genie app-server` process
//! of the session.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

/// The uid and PID of the process at the other end of a Unix socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub uid: u32,
    pub pid: u32,
}

/// The effective uid of this process.
pub fn own_uid() -> u32 {
    // SAFETY: `geteuid` has no argument and cannot fail.
    unsafe { libc::geteuid() }
}

/// The parent PID of this process.
pub fn own_parent_pid() -> u32 {
    // SAFETY: `getppid` has no argument and cannot fail.
    let p = unsafe { libc::getppid() };
    u32::try_from(p).unwrap_or(0)
}

/// `getpeereid` and `LOCAL_PEERPID` (macOS).
#[cfg(target_os = "macos")]
pub fn peer(stream: &UnixStream) -> io::Result<Peer> {
    let fd = stream.as_raw_fd();
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: `fd` is a valid socket and both out pointers are valid for the call.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `pid` and `len` are valid for the call and `len` is the size of `pid`.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Peer {
        uid,
        pid: u32::try_from(pid).map_err(|_| io::ErrorKind::InvalidData)?,
    })
}

/// `SO_PEERCRED` (Linux).
#[cfg(target_os = "linux")]
pub fn peer(stream: &UnixStream) -> io::Result<Peer> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for the call and `len` is the size of `cred`.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Peer {
        uid: cred.uid,
        pid: u32::try_from(cred.pid).map_err(|_| io::ErrorKind::InvalidData)?,
    })
}

/// No peer check on other Unix systems: the channel fails closed.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn peer(_stream: &UnixStream) -> io::Result<Peer> {
    Err(io::ErrorKind::Unsupported.into())
}

/// The parent PID of `pid` (`proc_pidinfo`, macOS).
#[cfg(target_os = "macos")]
pub fn parent_pid(pid: u32) -> Option<u32> {
    let pid = libc::c_int::try_from(pid).ok()?;
    // SAFETY: an all-zero `proc_bsdinfo` is a valid value (plain integers and byte arrays).
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is valid for `size` bytes for the call.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (n == size).then_some(info.pbi_ppid)
}

/// The parent PID of `pid` (`/proc/<pid>/stat`, Linux).
#[cfg(target_os = "linux")]
pub fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name is in parentheses and may hold spaces: read after the last `)`.
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// No parent lookup on other Unix systems: the channel fails closed.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn parent_pid(_pid: u32) -> Option<u32> {
    None
}
