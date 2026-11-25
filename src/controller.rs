//! Read-only queries to the local Mihomo controller.
use crate::{status, ROOT};
use curl::easy::{Easy, List};
use serde_json::Value;
use std::{
    fs::File,
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn get(local: &Value, path: &str, timeout: Duration) -> io::Result<Value> {
    let port = local["controller"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| io::Error::other("Invalid controller port"))?;
    let token = local["secret"]
        .as_str()
        .filter(|s| !s.contains(['\r', '\n', '\0']))
        .ok_or_else(|| io::Error::other("Invalid controller secret"))?;
    let mut headers = List::new();
    headers.append(&format!("Authorization: Bearer {token}"))?;
    let mut easy = Easy::new();
    easy.url(&format!("http://127.0.0.1:{port}{path}"))?;
    easy.proxy("")?;
    easy.http_headers(headers)?;
    easy.timeout(timeout)?;
    easy.fail_on_error(true)?;
    let mut bytes = Vec::new();
    {
        let mut transfer = easy.transfer();
        transfer.write_function(|chunk| {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Ok(0);
            }
            bytes.extend_from_slice(chunk);
            Ok(chunk.len())
        })?;
        transfer.perform()?;
    }
    if easy.response_code()? != 200 {
        return Err(io::Error::other("Unexpected controller response"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn wait_ready() -> io::Result<()> {
    let local: Value =
        serde_json::from_reader(File::open(Path::new(ROOT).join("configs/client.json"))?)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(1) {
            break;
        }
        if status::running()
            && get(
                &local,
                "/version",
                remaining.min(Duration::from_millis(500)),
            )
            .is_ok_and(|v| v["version"].is_string())
        {
            return Ok(());
        }
        thread::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    Err(io::Error::other(
        "Mihomo did not start. Check the local log.",
    ))
}

pub(crate) fn provider_counts(local: &Value) -> Option<(usize, usize)> {
    let json = get(local, "/providers/rules", Duration::from_secs(2)).ok()?;
    let providers = json["providers"].as_object()?;
    let mut populated = 0;
    for provider in providers.values() {
        if provider["ruleCount"].as_u64()? > 0 {
            populated += 1;
        }
    }
    Some((populated, providers.len()))
}
