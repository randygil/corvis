//! Ending the whole process tree of a `corvis posture` run.
//!
//! `cargo run` starts the application as its own child, and the application may start helpers. Killing only the
//! PID that was spawned leaves them running, and a helper that inherited stdout keeps the pipe open. So the run
//! is put in a Windows Job Object that kills its members when it is closed, or in its own Unix process group
//! that is killed with `SIGKILL`. The system calls are declared here directly, so the CLI keeps no dependencies.

use std::process::{Child, Command};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;

    #[repr(C)]
    struct BasicLimit {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        counters: [u64; 6],
    }

    #[repr(C)]
    struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *mut c_void, length: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// A Job Object that holds the child and everything it starts afterwards.
    pub struct Tree {
        job: Handle,
    }

    pub fn prepare(_command: &mut Command) {}

    pub fn install_interrupt_handler() {}

    pub fn interrupted() -> bool {
        false
    }

    impl Tree {
        /// Puts `child` in a new Job Object that kills its members when the object is closed. `None` when the
        /// operating system refuses (the caller then only has the child itself to kill).
        pub fn adopt(child: &Child) -> Option<Tree> {
            // SAFETY: plain Win32 calls with a zeroed, correctly laid out information block and the child's
            // process handle, which stays valid while `child` lives.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: ExtendedLimit = std::mem::zeroed();
                info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    &mut info as *mut ExtendedLimit as *mut c_void,
                    std::mem::size_of::<ExtendedLimit>() as u32,
                );
                if set == 0 || AssignProcessToJobObject(job, child.as_raw_handle() as Handle) == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(Tree { job })
            }
        }

        /// Ends every process of the job.
        pub fn kill(&self) {
            // SAFETY: `job` is a valid handle owned by `self`.
            unsafe {
                TerminateJobObject(self.job, 1);
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            // Closing the last handle kills the members (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
            // SAFETY: `job` is a valid handle owned by `self`, closed once.
            unsafe {
                TerminateJobObject(self.job, 1);
                CloseHandle(self.job);
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
        fn signal(signal: i32, handler: usize) -> usize;
    }

    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    const SIGKILL: i32 = 9;

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_signal(_signal: i32) {
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    /// The child leads its own process group, so the whole group can be killed, and a terminal Ctrl-C no
    /// longer reaches it by itself (the handler below forwards it).
    pub fn prepare(command: &mut Command) {
        command.process_group(0);
    }

    /// Ctrl-C and SIGTERM make the main loop stop and kill the group instead of orphaning it.
    pub fn install_interrupt_handler() {
        // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
        unsafe {
            signal(SIGINT, on_signal as extern "C" fn(i32) as usize);
            signal(SIGTERM, on_signal as extern "C" fn(i32) as usize);
        }
    }

    pub fn interrupted() -> bool {
        INTERRUPTED.load(Ordering::SeqCst)
    }

    /// The process group of the child.
    pub struct Tree {
        group: i32,
    }

    impl Tree {
        pub fn adopt(child: &Child) -> Option<Tree> {
            i32::try_from(child.id()).ok().map(|group| Tree { group })
        }

        /// Sends `SIGKILL` to every process of the group.
        pub fn kill(&self) {
            // SAFETY: a negative pid addresses the process group; the call has no memory effects.
            unsafe {
                kill(-self.group, SIGKILL);
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::*;

    pub struct Tree;

    pub fn prepare(_command: &mut Command) {}
    pub fn install_interrupt_handler() {}
    pub fn interrupted() -> bool {
        false
    }

    impl Tree {
        pub fn adopt(_child: &Child) -> Option<Tree> {
            None
        }
        pub fn kill(&self) {}
    }
}

pub use imp::{Tree, install_interrupt_handler, interrupted, prepare};
