use crate::{Locale, ROOT, RUN};
use serde_json::Value;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

pub(crate) fn running() -> bool {
    core_pid().is_some()
}

/// Linux /proc stat field 22 counts clock ticks since boot. The command name
/// is enclosed in parentheses and can itself contain spaces or parentheses.
pub(crate) fn birth(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(')')?;
    let mut fields = tail.split_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    fields.nth(18)?.parse().ok()
}

pub(crate) fn core_pid() -> Option<u32> {
    let stamp = fs::read_to_string(Path::new(RUN).join("core.pid")).ok()?;
    let mut fields = stamp.split_whitespace();
    let pid: u32 = fields.next()?.parse().ok()?;
    let expected: u64 = fields.next()?.parse().ok()?;
    if fields.next().is_some() || pid <= 1 || birth(pid)? != expected {
        return None;
    }
    let cmd = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    (cmd.split(|b| *b == 0).next()? == b"/etc/KokoroBox/mihomo").then_some(pid)
}

fn read_json(path: &Path) -> io::Result<Value> {
    Ok(serde_json::from_reader(fs::File::open(path)?)?)
}

pub(crate) fn show(l: &Locale) -> io::Result<i32> {
    let mut report = Vec::new();
    let code = report_status(l, &mut report)?;
    io::stdout().lock().write_all(&report)?;
    Ok(code)
}

fn report_status(l: &Locale, out: &mut Vec<u8>) -> io::Result<i32> {
    let root = Path::new(ROOT);
    let live = running();
    let word = if live { "running" } else { "stopped" };
    writeln!(
        out,
        "{}",
        if io::stdout().is_terminal() {
            l.tr(word)
        } else {
            word
        }
    )?;
    let local = read_json(&root.join("configs/client.json"))?;
    writeln!(out, "HTTP/SOCKS  127.0.0.1:{}", local["port"])?;
    let enabled = local["transparent"].as_bool() == Some(true);
    let route = if !live {
        if enabled {
            "On next start"
        } else {
            "Off"
        }
    } else if enabled || Path::new(RUN).join("host-route").exists() {
        let active = crate::routes::run("check").is_ok();
        if active {
            "Active"
        } else {
            "Incomplete"
        }
    } else {
        "Off"
    };
    writeln!(out, "{}  {}", l.tr("Local transparent proxy"), l.tr(route))?;
    let current = root.join("yamls/.kokoro/current");
    let profile = root.join("yamls/config.yaml");
    if profile.is_file()
        && fs::read_link(&profile).is_ok_and(|s| s == Path::new(".kokoro/current/config.yaml"))
    {
        let source = if current.join("url").is_file() {
            "External link"
        } else {
            "Kokoro account"
        };
        writeln!(out, "{}  {}", l.tr("Subscription source"), l.tr(source))?;
        let meta = current.join("active.json");
        if meta.is_file() {
            let info = read_json(&meta)?;
            for (key, label) in [
                ("plan", "Plan"),
                ("protocol", "Protocol"),
                ("rule_source", "Rule source"),
            ] {
                writeln!(
                    out,
                    "{} {}",
                    l.tr(label),
                    info[key].as_str().unwrap_or(l.tr("Default"))
                )?;
            }
        }
    } else {
        writeln!(
            out,
            "{}  {}",
            l.tr("Subscription source"),
            l.tr(if profile.is_file() {
                "Local file"
            } else {
                "None"
            })
        )?;
    }
    if live {
        let label = l.tr("Rule sets with rules / total");
        match crate::controller::provider_counts(&local) {
            Some((ready, total)) => writeln!(out, "{label}  {ready}/{total}"),
            None => writeln!(out, "{label}  {}", l.tr("Unavailable")),
        }?;
    }
    Ok(if live { 0 } else { 3 })
}
