//! Service managers run the hooks below without acquiring the command lock.
//! A public command holds that lock while the manager starts or stops the unit.
use crate::{routes, status, Locale, ROOT, RUN};
use fs2::FileExt;
use serde_json::Value;
use std::{
    env, fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{self, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

const SERVICE: &str = "kokorobox";
const UNIT: &str = "kokorobox.service";

fn log_file() -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(Path::new(RUN).join("core.log"))
}

fn service_lock() -> io::Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(Path::new(RUN).join("service.lock"))?;
    lock.lock_exclusive()?;
    Ok(lock)
}

fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    tmp.write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}

pub(crate) fn group_id() -> io::Result<u32> {
    let group = Command::new("getent").args(["group", SERVICE]).output()?;
    if group.status.success() {
        if let Some(id) = std::str::from_utf8(&group.stdout)
            .ok()
            .and_then(|s| s.trim_end().split(':').nth(2))
            .and_then(|s| s.parse::<u32>().ok())
        {
            return Ok(id);
        }
    }
    Err(io::Error::other("Cannot resolve the kokorobox group"))
}

fn check(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{}: {status}",
            command.get_program().to_string_lossy()
        )))
    }
}

fn prepare(l: &Locale) -> io::Result<()> {
    check(
        Command::new("python3")
            .current_dir(ROOT)
            .env("KOKOROBOX_UI_LANG", l.code)
            .args(["runtime.py", "prepare"]),
    )
}

fn run_core() -> io::Result<()> {
    let pid = process::id();
    let birth =
        status::birth(pid).ok_or_else(|| io::Error::other("Cannot identify core process"))?;
    let gid = group_id()?;
    let log = log_file()?;
    let mut cmd = Command::new(Path::new(ROOT).join("mihomo"));
    cmd.current_dir(ROOT)
        .args(["-d", ROOT, "-f"])
        .arg(Path::new(RUN).join("config.yaml"))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // nftables excludes this effective GID from capture, including DNS upstream sockets.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) == -1 || libc::setgid(gid) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    atomic_write(
        &Path::new(RUN).join("core.pid"),
        format!("{pid} {birth}\n").as_bytes(),
    )?;
    Err(cmd.exec())
}

static OPENRC_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_: libc::c_int) {
    OPENRC_STOP.store(true, Ordering::Relaxed);
}

// OpenRC executes start_post only once, while supervise-daemon may launch
// multiple generations. Each generation needs a fresh route after TUN recreation.
fn openrc_core() -> io::Result<()> {
    let mut command = Command::new(env::current_exe()?);
    command.args(["--service", "run-core"]);
    unsafe {
        if libc::signal(
            libc::SIGTERM,
            request_stop as *const () as libc::sighandler_t,
        ) == libc::SIG_ERR
        {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let parent = process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn()?;
    if let Err(error) = routes::run("start") {
        let _ = child.kill();
        child.wait()?;
        routes::run("stop")?;
        return Err(error);
    }
    let exit = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if OPENRC_STOP.load(Ordering::Relaxed) {
            // The tracked child is unreaped, so this PID cannot have been reused.
            signal_result(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) } != -1)?;
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(100));
    };
    routes::run("stop")?;
    if exit.success() || OPENRC_STOP.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err(io::Error::other(format!("Mihomo: {exit}")))
    }
}

fn supervise() -> io::Result<()> {
    let mut child = Command::new(env::current_exe()?)
        .args(["--service", "run-core"])
        .spawn()?;
    let pid = child.id();
    let birth = status::birth(pid);
    child.wait()?;
    // A rapid restart can already have installed a different core and routes.
    let _lock = service_lock()?;
    let stamp = Path::new(RUN).join("core.pid");
    if let Some(birth) = birth {
        if fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == format!("{pid} {birth}")) {
            routes::run("stop")?;
            fs::remove_file(stamp)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Manager {
    Systemd,
    OpenRc,
    Direct,
}

impl Manager {
    fn detect() -> Self {
        if Path::new("/run/systemd/system").is_dir() {
            Self::Systemd
        } else if Command::new("rc-status")
            .arg("-r")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            Self::OpenRc
        } else {
            Self::Direct
        }
    }

    fn start(self, l: &Locale) -> io::Result<()> {
        if status::running() {
            return Ok(());
        }
        match self {
            Self::Systemd => check(Command::new("systemctl").args(["start", UNIT])),
            Self::OpenRc => check(Command::new("rc-service").args([SERVICE, "start"])),
            Self::Direct => {
                // Recover routes left by an abruptly terminated supervisor before preparing a new TUN.
                routes::run("stop")?;
                prepare(l)?;
                let log = log_file()?;
                let mut cmd = Command::new(env::current_exe()?);
                cmd.args(["--service", "supervise"])
                    .current_dir(ROOT)
                    .stdin(Stdio::null())
                    .stdout(log.try_clone()?)
                    .stderr(log);
                // Give the detached supervisor and core their own session for SSH and startup rollback.
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let mut supervisor = cmd.spawn()?;
                if let Err(error) = routes::run("start") {
                    // We still own this unreaped child, so its session ID cannot be recycled.
                    let group = -(supervisor.id() as i32);
                    unsafe {
                        libc::kill(group, libc::SIGTERM);
                    }
                    // Core termination is asynchronous; wait before a mode rollback can launch another.
                    for _ in 0..80 {
                        if !status::running() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    unsafe {
                        libc::kill(group, libc::SIGKILL);
                    }
                    supervisor.wait()?;
                    routes::run("stop")?;
                    return Err(error);
                }
                Ok(())
            }
        }
    }

    fn stop(self) -> io::Result<()> {
        match self {
            Self::Systemd => check(Command::new("systemctl").args(["stop", UNIT])),
            Self::OpenRc => check(Command::new("rc-service").args([SERVICE, "stop"])),
            Self::Direct => {
                routes::run("stop")?;
                if let Some(pid) = status::core_pid() {
                    signal_core(pid)?;
                    for _ in 0..80 {
                        if status::core_pid() != Some(pid) {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    return Err(io::Error::other("Timed out stopping Mihomo"));
                }
                Ok(())
            }
        }
    }

    fn restart(self, l: &Locale) -> io::Result<()> {
        self.stop()?;
        self.start(l)
    }

    fn set_mode(self, enabled: bool, l: &Locale) -> io::Result<()> {
        let path = Path::new(ROOT).join("configs/client.json");
        let old = fs::read(&path)?;
        let mut config: Value = serde_json::from_slice(&old)?;
        let mode = config
            .get_mut("transparent")
            .filter(|v| v.is_boolean())
            .ok_or_else(|| io::Error::other("Invalid transparent proxy setting"))?;
        if mode.as_bool() == Some(enabled) {
            return Ok(());
        }
        if enabled
            && !(fs::metadata("/dev/net/tun").is_ok_and(|m| m.file_type().is_char_device())
                && Command::new("nft")
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success()))
        {
            return Err(io::Error::other(
                l.tr("Local transparent proxy requires nftables and /dev/net/tun.")
                    .to_owned(),
            ));
        }
        *mode = Value::Bool(enabled);
        let live = status::running();
        atomic_write(&path, &serde_json::to_vec(&config)?)?;
        if live && self.restart(l).is_err() {
            atomic_write(&path, &old)?;
            self.restart(l)?;
            return Err(io::Error::other(
                l.tr("Could not change proxy mode. Previous settings restored.")
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

fn signal_core(pid: u32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // Pin the process before rechecking its start time and executable. A recycled
        // numeric PID cannot redirect the signal delivered through the descriptor.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0u32) };
        if fd >= 0 {
            let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
            if status::core_pid() != Some(pid) {
                return Ok(());
            }
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    libc::SIGTERM,
                    std::ptr::null::<libc::siginfo_t>(),
                    0u32,
                )
            };
            return signal_result(result != -1);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => return Ok(()),
            Some(libc::ENOSYS | libc::EPERM) => (),
            _ => return Err(error),
        }
    }
    if status::core_pid() != Some(pid) {
        return Ok(());
    }
    signal_result(unsafe { libc::kill(pid as i32, libc::SIGTERM) } != -1)
}

fn signal_result(success: bool) -> io::Result<()> {
    if success {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(e)
    }
}

pub(crate) fn dispatch(args: &[&str], l: &Locale) -> io::Result<i32> {
    if let ["--service", hook] = args {
        match *hook {
            "prepare" => prepare(l)?,
            "run-core" => run_core()?,
            "openrc-core" => openrc_core()?,
            "supervise" => supervise()?,
            _ => return Ok(2),
        };
    } else {
        let _lock = service_lock()?;
        let manager = Manager::detect();
        match args {
            ["start"] => manager.start(l)?,
            ["stop"] => manager.stop()?,
            ["restart"] => manager.restart(l)?,
            ["transparent", "on"] => manager.set_mode(true, l)?,
            ["transparent", "off"] => manager.set_mode(false, l)?,
            _ => return Ok(2),
        }
    }
    Ok(0)
}
