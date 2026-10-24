use std::any::Any;
use std::io;
use std::path::PathBuf;

use hpcd_core::CpuSet;

pub struct Hold(#[allow(dead_code)] Box<dyn Any + Send>);

impl Hold {
    pub fn new(guard: impl Any + Send) -> Self {
        Self(Box::new(guard))
    }
}

pub trait System {
    fn read(&self, path: &str) -> io::Result<String>;
    fn write(&self, path: &str, value: &str) -> io::Result<()>;
    fn affinity(&self, tid: u32) -> io::Result<CpuSet>;
    fn set_affinity(&self, tid: u32, cpus: &CpuSet) -> io::Result<()>;
    fn hold_dma_latency(&self, us: u32) -> io::Result<Hold>;
}

pub struct Live {
    root: PathBuf,
}

impl Live {
    pub fn new() -> Self {
        Self { root: "/".into() }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

impl System for Live {
    fn read(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(self.resolve(path))
    }

    fn write(&self, path: &str, value: &str) -> io::Result<()> {
        std::fs::write(self.resolve(path), value)
    }

    #[cfg(target_os = "linux")]
    fn affinity(&self, tid: u32) -> io::Result<CpuSet> {
        let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::cpu_set_t>();
        if unsafe { libc::sched_getaffinity(tid as libc::pid_t, size, &mut set) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((0..libc::CPU_SETSIZE as u32)
            .filter(|c| unsafe { libc::CPU_ISSET(*c as usize, &set) })
            .collect())
    }

    #[cfg(target_os = "linux")]
    fn set_affinity(&self, tid: u32, cpus: &CpuSet) -> io::Result<()> {
        let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        for cpu in cpus.iter() {
            unsafe { libc::CPU_SET(cpu as usize, &mut set) };
        }
        let size = std::mem::size_of::<libc::cpu_set_t>();
        if unsafe { libc::sched_setaffinity(tid as libc::pid_t, size, &set) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn affinity(&self, _tid: u32) -> io::Result<CpuSet> {
        Err(io::ErrorKind::Unsupported.into())
    }

    #[cfg(not(target_os = "linux"))]
    fn set_affinity(&self, _tid: u32, _cpus: &CpuSet) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn hold_dma_latency(&self, us: u32) -> io::Result<Hold> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(self.resolve("/dev/cpu_dma_latency"))?;
        let value = i32::try_from(us).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        file.write_all(&value.to_ne_bytes())?;
        Ok(Hold::new(file))
    }
}
