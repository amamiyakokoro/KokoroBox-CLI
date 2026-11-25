//! GNU/Linux entry point. Service and subscription backends inherit the original TTY and exit status.
mod api;
mod auth;
mod controller;
mod routes;
mod service;
mod status;
mod subscription;

use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::{
    env, fs, io,
    path::Path,
    process::{self, Command},
};

const ROOT: &str = "/etc/KokoroBox";
const RUN: &str = "/run/kokorobox";
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Locale {
    code: &'static str,
    messages: Value,
}

impl Locale {
    fn load() -> Self {
        let raw = fs::read_to_string(format!("{ROOT}/configs/i18n.cfg"))
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                ["KOKOROBOX_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
                    .iter()
                    .find_map(|k| env::var(k).ok().filter(|s| !s.is_empty()))
            })
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .replace('-', "_");
        let (code, catalog) = if raw == "cht"
            || ["zh_tw", "zh_hk", "zh_mo", "zh_hant"]
                .iter()
                .any(|s| raw.starts_with(s))
        {
            ("cht", include_str!("../scripts/lang/cht/kokoro.json"))
        } else if raw == "chs" || raw == "zh" || raw.starts_with("zh_") {
            ("chs", include_str!("../scripts/lang/chs/kokoro.json"))
        } else {
            ("en", "{}")
        };
        Self {
            code,
            messages: serde_json::from_str(catalog).expect("invalid built-in catalog"),
        }
    }
    fn tr<'a>(&'a self, key: &'a str) -> &'a str {
        self.messages[key].as_str().unwrap_or(key)
    }
}

fn help(l: &Locale) {
    println!(
        "kokorobox                         {}",
        l.tr("Interactive menu")
    );
    println!("kokorobox login | whoami | logout | select | subscribe | sync");
    println!("kokorobox start | restart | stop | status | logs | test");
    println!(
        "kokorobox transparent on|off      {}",
        l.tr("Local transparent proxy")
    );
    println!(
        "kokorobox auto-sync on|off|show   {}",
        l.tr("Automatic hourly sync")
    );
    println!("kokorobox uninstall");
}

fn dispatch(args: &[&str], l: &Locale) -> io::Result<i32> {
    match args {
        ["--version"] => {
            println!("kokorobox {VERSION}");
            return Ok(0);
        }
        ["-h" | "--help" | "help"] => {
            help(l);
            return Ok(0);
        }
        _ => {}
    }
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("{}", l.tr("Run with sudo to manage KokoroBox-CLI."));
        return Ok(1);
    }
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(RUN)?;
    let mut cmd = match args {
        [] | ["-l"] | ["-u" | "uninstall"] => {
            let mut c = Command::new("./menu.sh");
            c.args(args);
            c
        }
        [op @ ("login" | "whoami" | "logout" | "select" | "subscribe" | "sync")] => {
            return Ok(match subscription::run(op, l) {
                Ok(()) => 0,
                Err(e) => {
                    e.show(l);
                    1
                }
            });
        }
        ["start" | "stop" | "restart"]
        | ["transparent", "on" | "off"]
        | ["--service", "prepare" | "run-core" | "supervise" | "openrc-core"] => {
            return service::dispatch(args, l);
        }
        ["--host-route", op @ ("start" | "stop" | "refresh" | "check")] => {
            routes::run(op)?;
            return Ok(0);
        }
        ["auto-sync"] | ["auto-sync", "on" | "off" | "show"] => {
            let mut c = Command::new("python3");
            c.args(["schedule.py", args.get(1).unwrap_or(&"show")]);
            c
        }
        ["test"] => {
            let mut c = Command::new("python3");
            c.args(["runtime.py", "probe"]);
            c
        }
        ["logs"] => {
            let log = Path::new(RUN).join("core.log");
            if !log.exists() {
                println!("{}", l.tr("No local log yet."));
                return Ok(0);
            }
            let mut c = Command::new("tail");
            c.args(["-n", "60"]).arg(log);
            c
        }
        ["status"] => return status::show(l),
        ["--core-running"] => return Ok(if status::running() { 0 } else { 1 }),
        _ => {
            help(l);
            return Ok(2);
        }
    };
    Err(cmd
        .current_dir(ROOT)
        .env("KOKOROBOX_UI_LANG", l.code)
        .exec())
}
fn main() {
    // Runtime files and credentials created by the CLI and its children are private.
    unsafe { libc::umask(0o077) };
    let args: Vec<_> = env::args().skip(1).collect();
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let locale = Locale::load();
    let code = dispatch(&args, &locale).unwrap_or_else(|e| {
        eprintln!("kokorobox: {}", locale.tr(&e.to_string()));
        1
    });
    process::exit(code);
}
