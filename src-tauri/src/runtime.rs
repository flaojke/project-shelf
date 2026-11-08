//! All native commands use argument arrays. No shell and no inherited console.
//! Long-lived jobs deliberately do NOT kill children when Shelf exits: the user
//! may explicitly choose to leave projects running. Ownership never persists.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

// Test-only tripwire, not a runtime permission switch. Production callers use
// their normal setup/live-action validation before reaching native operations.
// Unit tests fail closed here and can assert that incomplete profiles never
// even attempted to cross the native boundary.
#[cfg(test)]
static NATIVE_EFFECT_ATTEMPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn native_effect_boundary() -> Result<(), String> {
    NATIVE_EFFECT_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Err("Native OS effects are disabled in unit tests.".into())
}

#[cfg(not(test))]
pub(crate) fn native_effect_boundary() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn native_effect_attempts() -> usize {
    NATIVE_EFFECT_ATTEMPTS.load(std::sync::atomic::Ordering::SeqCst)
}

pub struct ToolOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub struct ManagedProcess {
    child: Child,
    #[cfg(windows)]
    job: Option<windows_job::Job>,
}
impl ManagedProcess {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    /// Check the launcher itself, not all detached descendants in its Job.
    /// A successful wrapper exit may intentionally leave a server alive.
    pub fn root_exit(&mut self) -> Result<Option<(bool, Option<i32>)>, String> {
        self.child
            .try_wait()
            .map(|status| status.map(|s| (s.success(), s.code())))
            .map_err(|_| "Не удалось проверить завершение штатного launcher.".into())
    }
    pub fn owns_listener(&self, port: u16) -> Result<bool, String> {
        #[cfg(windows)]
        {
            self.job
                .as_ref()
                .ok_or("Процесс не изолирован в Windows Job.")?
                .owns_listener(port)
        }
        #[cfg(not(windows))]
        {
            let _ = port;
            Ok(false)
        }
    }
    pub fn exited(&mut self) -> Result<bool, String> {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            return job.empty();
        }
        self.child
            .try_wait()
            .map(|s| s.is_some())
            .map_err(|_| "Не удалось проверить свой процесс.".into())
    }
    /// Vite's own bridge closes the server and waits for in-flight requests.
    /// A failed write or timeout leaves the project running for an explicit
    /// retry. This path intentionally has no forced-termination fallback.
    pub async fn graceful_shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        if self.exited()? {
            return Ok(());
        }
        let input = self
            .child
            .stdin
            .as_mut()
            .ok_or("Нет безопасного канала остановки. Сервер оставлен работающим.")?;
        input.write_all(b"shutdown\n").and_then(|_|input.flush()).map_err(|_|"Сервер не принял корректную остановку. Он оставлен работающим; принудительного завершения нет.")?;
        wait_graceful_exit(|| self.exited(), timeout, Duration::from_millis(100)).await
    }
    // Only the finite CLI watchdog may terminate its own tree. Project Stop
    // must use a graceful adapter-specific operation instead.
    fn terminate_tool_tree(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                job.terminate()?;
            } else {
                self.child
                    .kill()
                    .map_err(|_| "Не удалось завершить собственный CLI-процесс.")?;
            }
        }
        #[cfg(not(windows))]
        {
            self.child
                .kill()
                .map_err(|_| "Не удалось завершить собственный процесс.")?;
        }
        let _ = self.child.wait();
        Ok(())
    }
}

async fn wait_graceful_exit(
    mut check: impl FnMut() -> Result<bool, String>,
    timeout: Duration,
    interval: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check()? {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("Сервер не завершился за отведённое время. Он оставлен работающим; принудительная остановка не выполнялась.".into());
        }
        tokio::time::sleep(remaining.min(interval)).await;
    }
}

fn spawn(
    executable: &str,
    args: &[String],
    cwd: Option<&Path>,
    capture: bool,
    kill_on_close: bool,
    control_stdin: bool,
    child_env: &[(String, String)],
) -> Result<ManagedProcess, String> {
    native_effect_boundary()?;
    if executable.chars().any(char::is_control) || args.iter().any(|s| s.contains('\0')) {
        return Err("Некорректные аргументы процесса.".into());
    }
    let mut command = Command::new(executable);
    command.args(args).stdin(if control_stdin {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    for (key, value) in child_env {
        if ![
            "ANDROID_AVD_HOME",
            "ANDROID_USER_HOME",
            "ANDROID_SDK_ROOT",
            "ANDROID_HOME",
        ]
        .contains(&key.as_str())
        {
            return Err("Запрещённая переменная процесса.".into());
        }
        crate::validate::absolute_path(value)?;
        command.env(key, value);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if capture {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    // ADB may start its shared daemon. Never put that daemon in a kill-on-close
    // job, nor terminate it as a side effect of a completed/timed-out query.
    let shared_adb = capture
        && Path::new(executable)
            .file_name()
            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("adb.exe") || n == "adb")
            .unwrap_or(false);
    let shared_daemon = shared_adb
        || (capture
            && Path::new(executable).file_name().is_some_and(|n| {
                n.to_string_lossy().eq_ignore_ascii_case("docker.exe") || n == "docker"
            })
            && args
                .iter()
                .take(2)
                .map(String::as_str)
                .eq(["desktop", "start"]));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        command.creation_flags(CREATE_NO_WINDOW | if shared_daemon { 0 } else { CREATE_SUSPENDED });
    }
    let mut child = command.spawn().map_err(|_| {
        format!(
            "Не удалось запустить инструмент {}. Проверьте установку и путь.",
            Path::new(executable)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    })?;
    #[cfg(windows)]
    {
        if shared_daemon {
            return Ok(ManagedProcess { child, job: None });
        }
        // Attach while suspended, so no child can escape the job before setup.
        match windows_job::Job::attach(&child, kill_on_close) {
            Ok(job) => {
                if let Err(error) = job.resume(child.id()) {
                    let _ = job.terminate();
                    let _ = child.wait();
                    return Err(error);
                }
                Ok(ManagedProcess {
                    child,
                    job: Some(job),
                })
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (kill_on_close, shared_daemon);
        Ok(ManagedProcess { child })
    }
}

pub fn spawn_managed(
    executable: &str,
    args: &[String],
    cwd: &Path,
) -> Result<ManagedProcess, String> {
    spawn(executable, args, Some(cwd), false, false, false, &[])
}
pub fn spawn_controlled(
    executable: &str,
    args: &[String],
    cwd: &Path,
) -> Result<ManagedProcess, String> {
    spawn(executable, args, Some(cwd), false, false, true, &[])
}
pub fn spawn_managed_env(
    executable: &str,
    args: &[String],
    cwd: &Path,
    child_env: &[(String, String)],
) -> Result<ManagedProcess, String> {
    spawn(executable, args, Some(cwd), false, false, false, child_env)
}

pub async fn run_tool(
    executable: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<ToolOutput, String> {
    native_effect_boundary()?;
    let executable = executable.to_string();
    let args = args.to_vec();
    let cwd = cwd.map(Path::to_path_buf);
    // The blocking task owns a kill-on-close job. Even if its async caller is
    // cancelled, this finite watchdog still terminates only its own command.
    tokio::task::spawn_blocking(move || {
        let mut process = spawn(&executable, &args, cwd.as_deref(), true, true, false, &[])?;
        fn drain(reader: impl Read + Send + 'static, limit: usize) -> std::sync::mpsc::Receiver<Vec<u8>> {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut reader = reader; let mut output = Vec::new(); let mut buffer = [0_u8; 4096];
                loop { match reader.read(&mut buffer) { Ok(0) | Err(_) => break, Ok(n) => {
                    let remaining = limit.saturating_sub(output.len()); output.extend_from_slice(&buffer[..n.min(remaining)]);
                } } }
                let _ = tx.send(output);
            }); rx
        }
        let stdout = process.child.stdout.take().map(|p| drain(p, 128 * 1024));
        let stderr = process.child.stderr.take().map(|p| drain(p, 8 * 1024));
        let deadline = Instant::now() + timeout;
        let status = loop {
            match process.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
                Ok(None) => { let _ = process.terminate_tool_tree(); return Err("Инструмент превысил время ожидания; остановлен только собственный CLI-процесс.".into()); }
                Err(_) => { let _ = process.terminate_tool_tree(); return Err("Не удалось дождаться собственного инструмента.".into()); }
            }
        };
        drop(process); // bounded tool jobs must not leave descendants behind
        let read = |rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>| rx.and_then(|r| r.recv_timeout(Duration::from_millis(300)).ok()).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
        Ok(ToolOutput { success: status.success(), exit_code: status.code(), stdout: read(stdout), stderr: read(stderr) })
    }).await.map_err(|_| "Сбой выполнения инструмента.".to_string())?
}

pub async fn open_browser(value: &str) -> Result<(), String> {
    native_effect_boundary()?;
    let url = crate::validate::loopback_url(value)?.to_string();
    // Use the system URL association directly, no cmd/start shell expansion.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        Command::new("explorer.exe")
            .arg(url)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Не удалось открыть браузер.")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("Открытие браузера реализовано для Windows.".into())
    }
}

#[cfg(test)]
mod graceful_tests {
    use super::*;
    #[tokio::test]
    async fn graceful_wait_observes_completion_before_success() {
        let mut probes = 0;
        wait_graceful_exit(
            || {
                probes += 1;
                Ok(probes >= 3)
            },
            Duration::from_millis(100),
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        assert_eq!(probes, 3);
    }
    #[tokio::test]
    async fn graceful_timeout_returns_error_and_does_not_claim_stopped() {
        let mut probes = 0;
        let result = wait_graceful_exit(
            || {
                probes += 1;
                Ok(false)
            },
            Duration::from_millis(5),
            Duration::from_millis(1),
        )
        .await;
        assert!(result.unwrap_err().contains("оставлен работающим"));
        assert!(probes >= 2);
    }
    #[tokio::test]
    async fn graceful_check_error_is_preserved_without_retry_or_force() {
        let mut probes = 0;
        let result = wait_graceful_exit(
            || {
                probes += 1;
                Err("identity check failed".into())
            },
            Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(result.unwrap_err(), "identity check failed");
        assert_eq!(probes, 1);
    }
}

#[cfg(windows)]
mod windows_job {
    use std::{
        mem::{size_of, zeroed},
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::Child,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    pub struct Job {
        handle: OwnedHandle,
    }
    impl Job {
        pub fn owns_listener(&self, port: u16) -> Result<bool, String> {
            use windows_sys::Win32::{
                NetworkManagement::IpHelper::{
                    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
                },
                Networking::WinSock::AF_INET,
                System::{
                    JobObjects::IsProcessInJob,
                    Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
                },
            };
            let mut size = 0_u32;
            unsafe {
                GetExtendedTcpTable(
                    std::ptr::null_mut(),
                    &mut size,
                    0,
                    AF_INET as u32,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                );
            }
            if size < 4 || size > 16 * 1024 * 1024 {
                return Err("Не удалось прочитать владельца локального порта.".into());
            }
            let mut buffer = vec![0_u8; size as usize];
            if unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr() as *mut _,
                    &mut size,
                    0,
                    AF_INET as u32,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            } != 0
            {
                return Err("Не удалось подтвердить владельца локального порта.".into());
            }
            let count = unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const u32) } as usize;
            if count > (buffer.len() - 4) / size_of::<MIB_TCPROW_OWNER_PID>() {
                return Err("Некорректная таблица владельцев портов.".into());
            }
            for index in 0..count {
                let row = unsafe {
                    std::ptr::read_unaligned(
                        buffer
                            .as_ptr()
                            .add(4 + index * size_of::<MIB_TCPROW_OWNER_PID>())
                            as *const MIB_TCPROW_OWNER_PID,
                    )
                };
                if u16::from_be(row.dwLocalPort as u16) != port {
                    continue;
                }
                if row.dwLocalAddr != 0 && row.dwLocalAddr != u32::from_ne_bytes([127, 0, 0, 1]) {
                    continue;
                }
                let raw =
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, row.dwOwningPid) };
                if raw.is_null() {
                    return Err("Windows не разрешил проверить владельца порта.".into());
                }
                let process = unsafe { OwnedHandle::from_raw_handle(raw) };
                let mut ours = 0;
                if unsafe {
                    IsProcessInJob(
                        process.as_raw_handle(),
                        self.handle.as_raw_handle(),
                        &mut ours,
                    )
                } == 0
                {
                    return Err("Не удалось проверить принадлежность процесса Windows Job.".into());
                }
                if ours != 0 {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        pub fn empty(&self) -> Result<bool, String> {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.handle.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err("Не удалось проверить собственный Windows Job.".into());
            }
            Ok(info.ActiveProcesses == 0)
        }
        pub fn attach(child: &Child, kill_on_close: bool) -> Result<Self, String> {
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err("Не удалось создать Windows Job Object.".into());
            }
            let job = Self {
                handle: unsafe { OwnedHandle::from_raw_handle(raw) },
            };
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            if kill_on_close {
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            }
            if unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } == 0
                || unsafe { AssignProcessToJobObject(raw, child.as_raw_handle()) } == 0
            {
                return Err("Не удалось изолировать дерево процесса; запуск отменён.".into());
            }
            Ok(job)
        }
        pub fn resume(&self, pid: u32) -> Result<(), String> {
            let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if raw == INVALID_HANDLE_VALUE {
                return Err("Не удалось возобновить изолированный процесс.".into());
            }
            let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
            let mut entry: THREADENTRY32 = unsafe { zeroed() };
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut found = false;
            let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    let thread =
                        unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    if !thread.is_null() {
                        let result = unsafe { ResumeThread(thread) };
                        unsafe {
                            CloseHandle(thread);
                        }
                        if result != u32::MAX {
                            found = true;
                        }
                    }
                }
                more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
            }
            if !found {
                return Err("Не удалось возобновить собственный процесс; запуск отменён.".into());
            }
            Ok(())
        }
        pub fn terminate(&self) -> Result<(), String> {
            if unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) } == 0 {
                return Err("Windows не подтвердил остановку собственного дерева процесса.".into());
            }
            Ok(())
        }
    }
}
