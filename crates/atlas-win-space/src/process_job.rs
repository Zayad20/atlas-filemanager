//! A provider's descendants must not outlive it after a crash or forced exit.
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

pub struct ChildJob(HANDLE);
impl ChildJob {
    /// `process` is a borrowed OS process handle, normally from the child just spawned.
    pub fn contain(process: isize) -> std::io::Result<Self> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let guard = Self(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job, process) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(guard)
        }
    }
}
impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_the_job_terminates_only_its_owned_child() {
        use std::os::windows::{io::AsRawHandle, process::CommandExt};
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let job = ChildJob::contain(child.as_raw_handle() as isize).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        // Windows can use exit code zero when terminating a job. The contract
        // is prompt process exit, not an application-defined exit code.
        for _ in 0..100 {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("owned child survived closing its job");
    }
}
