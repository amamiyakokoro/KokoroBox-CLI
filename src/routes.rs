//! Linux host-only capture. The persistent marker is also understood by the 0.2 installer.
use crate::{controller, service, RUN};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

const TABLE: &str = "18992";
const MARK: &str = "0x4b42";
const DEVICE: &str = "kbox0";
const NFT_TABLE: &str = "kokorobox_host";
const INCOMPLETE: &str = "Local transparent proxy routing is incomplete.";
const FAILED: &str = "Could not apply local transparent proxy routing.";

fn output(program: &str, args: &[&str]) -> io::Result<Output> {
    Command::new(program).args(args).env("LC_ALL", "C").output()
}

fn stdout(program: &str, args: &[&str]) -> io::Result<String> {
    let result = output(program, args)?;
    if !result.status.success() {
        return Err(io::Error::other(FAILED));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn families(marker: &Path) -> io::Result<Vec<&'static str>> {
    match fs::read_to_string(marker)?.trim() {
        "4" => Ok(vec!["-4"]),
        "4 6" => Ok(vec!["-4", "-6"]),
        _ => Err(io::Error::other(INCOMPLETE)),
    }
}

fn nft_table_exists() -> io::Result<bool> {
    let tables = stdout("nft", &["list", "tables"])?;
    Ok(tables
        .lines()
        .any(|line| line.trim() == "table inet kokorobox_host"))
}

fn rules(family: &str) -> io::Result<String> {
    stdout("ip", &[family, "-N", "rule", "show"])
}

fn routes(family: &str) -> io::Result<String> {
    let result = output("ip", &[family, "-N", "route", "show", "table", TABLE])?;
    if result.status.success() {
        return Ok(String::from_utf8_lossy(&result.stdout).into_owned());
    }
    // An unallocated FIB table has no routes. Permission and tool errors must propagate.
    if String::from_utf8_lossy(&result.stderr).contains("FIB table does not exist") {
        return Ok(String::new());
    }
    Err(io::Error::other(FAILED))
}

fn has_rule(text: &str) -> bool {
    text.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.first() == Some(&"18992:")
            && fields.windows(2).any(|w| w == ["lookup", TABLE])
            && fields.windows(2).any(|w| w == ["iif", "lo"])
            && fields
                .windows(2)
                .any(|w| w[0] == "fwmark" && matches!(w[1], "0x4b42" | "0x4b42/0xffffffff"))
    })
}

fn has_route(text: &str) -> bool {
    text.lines().any(|line| {
        line.split_whitespace()
            .take(3)
            .eq(["default", "dev", DEVICE])
    })
}

fn check(marker: &Path) -> io::Result<()> {
    let families = families(marker)?;
    let tun = stdout(
        "ip",
        &["-o", "link", "show", "up", "dev", DEVICE, "type", "tun"],
    )?;
    let chain = stdout("nft", &["-n", "list", "chain", "inet", NFT_TABLE, "output"])?;
    if tun.trim().is_empty() || !is_capture_chain(&chain) {
        return Err(io::Error::other(INCOMPLETE));
    }
    for family in families {
        if !has_rule(&rules(family)?) || !has_route(&routes(family)?) {
            return Err(io::Error::other(INCOMPLETE));
        }
    }
    Ok(())
}

fn is_capture_chain(chain: &str) -> bool {
    let marked = chain
        .lines()
        .filter_map(|line| line.split_once("meta mark set 0x"))
        .any(|(_, value)| {
            value
                .split_whitespace()
                .next()
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                == Some(0x4b42)
        });
    marked
        && chain.contains("type route hook output")
        && (chain.contains("ct direction reply return") || chain.contains("ct direction 1 return"))
        && chain.contains("meta skgid ")
        && chain.contains("0x40000000")
}

#[derive(Clone, Copy)]
enum Policy {
    Route,
    Rule,
}

impl Policy {
    fn change(self, family: &str, action: &str) -> io::Result<()> {
        let result = match self {
            Self::Route => output(
                "ip",
                &[
                    family, "route", action, "default", "dev", DEVICE, "table", TABLE,
                ],
            )?,
            Self::Rule => output(
                "ip",
                &[
                    family, "rule", action, "pref", TABLE, "iif", "lo", "fwmark", MARK, "table",
                    TABLE,
                ],
            )?,
        };
        if result.status.success() {
            return Ok(());
        }
        // The kernel can remove a route when Mihomo closes the TUN. Stopping twice is harmless.
        if action == "del" {
            let present = match self {
                Self::Route => has_route(&routes(family)?),
                Self::Rule => has_rule(&rules(family)?),
            };
            if !present {
                return Ok(());
            }
        }
        Err(io::Error::other(FAILED))
    }
}

fn stop(marker: &Path) -> io::Result<()> {
    if !marker.try_exists()? {
        return Ok(());
    }
    let families = families(marker)?;
    if nft_table_exists()? {
        let table = stdout("nft", &["-a", "-n", "list", "table", "inet", NFT_TABLE])?;
        // Identify this generation even if its capture chain was edited, and recognize
        // the original 0.2 rules during upgrades. Preserve a replacement table.
        if table
            .lines()
            .any(|line| line.trim_start().starts_with("chain kokorobox_owner {"))
            || is_capture_chain(&table)
        {
            let handle = table
                .lines()
                .next()
                .and_then(|line| line.rsplit_once("# handle "))
                .and_then(|(_, id)| id.trim().parse::<u64>().ok())
                .ok_or_else(|| io::Error::other(FAILED))?;
            stdout(
                "nft",
                &["delete", "table", "inet", "handle", &handle.to_string()],
            )?;
        }
    }
    for family in families {
        Policy::Rule.change(family, "del")?;
        Policy::Route.change(family, "del")?;
    }
    fs::remove_file(marker)
}

fn install_chain(gid: u32) -> io::Result<()> {
    // Both commands are in one nft transaction. "create" rejects a concurrent table owner.
    let config = format!(
        r#"create table inet {NFT_TABLE}
table inet {NFT_TABLE} {{
    chain kokorobox_owner {{ }}
    chain output {{
        type route hook output priority mangle; policy accept;
        meta skgid {gid} return
        meta mark != 0 return
        ct direction reply return
        fib daddr type {{ local, broadcast, multicast }} return
        ip daddr 169.254.0.0/16 return
        ip6 daddr fe80::/10 return
        # Keep established TCP connections (including SSH) on their original route.
        meta l4proto tcp ct mark & 0x40000000 == 0 tcp flags & (syn | ack) != syn return
        meta l4proto tcp ct mark set ct mark | 0x40000000
        meta l4proto {{ tcp, udp }} meta mark set {MARK}
    }}
}}
"#
    );
    let mut nft = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .spawn()?;
    let sent = nft.stdin.take().unwrap().write_all(config.as_bytes());
    let status = nft.wait()?;
    sent?;
    if !status.success() {
        return Err(io::Error::other(FAILED));
    }
    Ok(())
}

fn install(marker: &Path) -> io::Result<()> {
    let gid = service::group_id()?;
    let ipv6 = match fs::read_to_string("/proc/sys/net/ipv6/conf/all/disable_ipv6") {
        Ok(s) => s.trim() == "0",
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    let active = if ipv6 { &["-4", "-6"][..] } else { &["-4"][..] };
    stop(marker)?;
    if nft_table_exists()? {
        return Err(io::Error::other(
            "The KokoroBox nftables table is already in use.",
        ));
    }
    for &family in active {
        if rules(family)?.lines().any(|s| s.starts_with("18992:"))
            || !routes(family)?.trim().is_empty()
        {
            return Err(io::Error::other(
                "Routing table or priority 18992 is in use.",
            ));
        }
    }
    let mut state = tempfile::NamedTempFile::new_in(RUN)?;
    writeln!(state, "{}", if ipv6 { "4 6" } else { "4" })?;
    state.persist(marker).map_err(|e| e.error)?;
    let mut installed = Vec::new();
    let result = (|| {
        for &family in active {
            for object in [Policy::Route, Policy::Rule] {
                object.change(family, "add")?;
                installed.push((family, object));
            }
        }
        install_chain(gid)
    })();
    if result.is_err() {
        // Roll back only successful additions. In particular a failed exclusive nft
        // creation never grants us ownership of an existing table.
        for (family, object) in installed.into_iter().rev() {
            object.change(family, "del")?;
        }
        fs::remove_file(marker)?;
    }
    result
}

pub(crate) fn run(op: &str) -> io::Result<()> {
    let run = Path::new(RUN);
    let marker = run.join("host-route");
    if op == "check" {
        return check(&marker);
    }
    if op == "start" {
        controller::wait_ready()?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(run.join("host-route.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    if op == "stop" {
        return stop(&marker);
    }
    if !matches!(op, "start" | "refresh") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid host route operation",
        ));
    }
    if !run.join("transparent").try_exists()? {
        return Ok(());
    }
    for _ in 0..50 {
        if output("ip", &["link", "show", "dev", DEVICE])?
            .status
            .success()
        {
            if op == "start" {
                // OpenRC can report startup from both its health hook and its supervisor.
                if marker.try_exists()? && check(&marker).is_ok() {
                    return Ok(());
                }
                return install(&marker);
            }
            for family in families(&marker)? {
                let existing = routes(family)?;
                if existing.trim().is_empty() {
                    Policy::Route.change(family, "add")?;
                } else if !has_route(&existing) {
                    return Err(io::Error::other(
                        "Routing table or priority 18992 is in use.",
                    ));
                }
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(io::Error::other(
        "Local transparent proxy requires nftables and /dev/net/tun.",
    ))
}
