//! UFFD (userfaultfd) handshake — replicates src/vmm/src/persist.rs behaviour.
//!
//! On snapshot load with backend_type: Uffd, real Firecracker:
//!   1. Allocates anonymous guest memory regions
//!   2. Creates a userfaultfd and registers the regions
//!   3. Connects to backend_path (Unix socket)
//!   4. Sends JSON Vec<GuestRegionUffdMapping> + UFFD fd via SCM_RIGHTS

use std::io::IoSlice;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use tracing::{info, warn};

use crate::api_types::GuestRegionUffdMapping;

/// Holds the UFFD fd and the allocated memory region.
/// The caller takes ownership of the memory via `ptr`/`size`
/// and must call `std::mem::forget(UffdState)` after extracting them
/// to prevent double-free (GuestMemory will own the mapping).
pub struct UffdState {
    pub uffd_fd: RawFd,
    pub ptr: *mut u8,
    pub size: usize,
}

unsafe impl Send for UffdState {}
unsafe impl Sync for UffdState {}

impl Drop for UffdState {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.uffd_fd);
            libc::munmap(self.ptr as *mut libc::c_void, self.size);
        }
    }
}

pub fn handshake(
    backend_path: &Path,
    mem_size_mib: usize,
    page_size: usize,
) -> Result<UffdState, Box<dyn std::error::Error>> {
    let mem_size = mem_size_mib * 1024 * 1024;

    let anon_mem = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            mem_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if anon_mem == libc::MAP_FAILED {
        return Err(format!("mmap: {}", std::io::Error::last_os_error()).into());
    }

    let uffd_fd = unsafe {
        libc::syscall(libc::SYS_userfaultfd, libc::O_CLOEXEC | libc::O_NONBLOCK)
    };
    if uffd_fd < 0 {
        unsafe { libc::munmap(anon_mem, mem_size); }
        return Err(format!("userfaultfd: {}", std::io::Error::last_os_error()).into());
    }
    let uffd_fd = uffd_fd as RawFd;

    #[repr(C)]
    struct UffdioApi { api: u64, features: u64, ioctls: u64 }

    let mut api = UffdioApi { api: 0xAA, features: 0x20, ioctls: 0 };
    if unsafe { libc::ioctl(uffd_fd, 0xc018aa3f_u64 as libc::c_ulong, &mut api) } < 0 {
        unsafe { libc::close(uffd_fd); libc::munmap(anon_mem, mem_size); }
        return Err(format!("UFFDIO_API: {}", std::io::Error::last_os_error()).into());
    }

    #[repr(C)]
    struct UffdioRegister { start: u64, len: u64, mode: u64, ioctls: u64 }

    let mut reg = UffdioRegister {
        start: anon_mem as u64, len: mem_size as u64,
        mode: 0x01, ioctls: 0,
    };
    if unsafe { libc::ioctl(uffd_fd, 0xc020aa00_u64 as libc::c_ulong, &mut reg) } < 0 {
        warn!(err = %std::io::Error::last_os_error(), "UFFDIO_REGISTER failed (non-fatal)");
    }

    let mappings = vec![GuestRegionUffdMapping {
        base_host_virt_addr: anon_mem as u64,
        size: mem_size,
        offset: 0,
        page_size,
    }];
    let json = serde_json::to_string(&mappings)?;

    info!(path = %backend_path.display(), "Connecting to UFFD handler");
    let stream = UnixStream::connect(backend_path)?;
    send_with_fd(&stream, json.as_bytes(), uffd_fd)?;
    std::mem::forget(stream);

    info!(uffd_fd, mem_size, "UFFD handshake complete");
    Ok(UffdState { uffd_fd, ptr: anon_mem as *mut u8, size: mem_size })
}

fn send_with_fd(stream: &UnixStream, data: &[u8], fd: RawFd) -> std::io::Result<()> {
    let iov = [IoSlice::new(data)];
    let cmsg_space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let cmsg_len = unsafe { libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let hdr = libc::msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: iov.as_ptr() as *mut libc::iovec,
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_mut_ptr() as *mut libc::c_void,
        msg_controllen: cmsg_space,
        msg_flags: 0,
    };

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&hdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = cmsg_len;
        std::ptr::copy_nonoverlapping(fd.to_ne_bytes().as_ptr(), libc::CMSG_DATA(cmsg), 4);

        if libc::sendmsg(stream.as_raw_fd(), &hdr, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
