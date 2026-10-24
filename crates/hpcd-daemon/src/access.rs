use std::ffi::{CStr, CString};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

pub fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}

pub fn group_id(name: &str) -> Option<u32> {
    let name = CString::new(name).ok()?;
    let mut group: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut found: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            &mut group,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    (rc == 0 && !found.is_null()).then_some(group.gr_gid)
}

pub fn groups_of(uid: u32) -> Vec<u32> {
    let mut user: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut user, buf.as_mut_ptr(), buf.len(), &mut found) };
    if rc != 0 || found.is_null() {
        return Vec::new();
    }
    let name = unsafe { CStr::from_ptr(user.pw_name) };
    let mut count: libc::c_int = 64;
    loop {
        let mut gids = vec![0 as libc::gid_t; count as usize];
        let before = count;
        let rc = unsafe {
            libc::getgrouplist(name.as_ptr(), user.pw_gid, gids.as_mut_ptr(), &mut count)
        };
        if rc >= 0 {
            gids.truncate(count as usize);
            return gids;
        }
        if count <= before {
            count = before * 2;
        }
    }
}
