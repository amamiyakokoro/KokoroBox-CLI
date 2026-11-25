use crate::api::{self, Body, Error, Format, Result, API};
use crate::{Locale, ROOT};
use serde_json::{json, Value};
use std::os::unix::fs::DirBuilderExt;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const SESSION: &str = "auth/session.json";
const CLIENT: &str = "kokorobox-shell";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const TIMEOUT: Duration = Duration::from_secs(30);

fn now() -> Result<f64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Local)?
        .as_secs_f64())
}
fn token<'a>(doc: &'a Value, key: &str) -> Result<&'a str> {
    doc[key]
        .as_str()
        .filter(|s| api::opaque(s))
        .ok_or(Error::Message(
            "Invalid Kokoro token response. Sign in again.",
        ))
}
fn save_tokens(bytes: &[u8]) -> Result<Value> {
    let t = api::object(bytes)?;
    let invalid = || Error::Message("Invalid Kokoro token response. Sign in again.");
    if !t["token_type"]
        .as_str()
        .unwrap_or("")
        .eq_ignore_ascii_case("bearer")
    {
        return Err(invalid());
    }
    let now = now()?;
    let expires = |key: &str| {
        t[key]
            .as_u64()
            .filter(|n| *n > 0)
            .map(|n| now + n as f64)
            .ok_or_else(invalid)
    };
    let session = json!({"access_token":token(&t,"access_token")?, "refresh_token":token(&t,"refresh_token")?,
        "access_expires":expires("expires_in")?, "refresh_expires":expires("refresh_expires_in")?});
    let dest = Path::new(ROOT).join(SESSION);
    let dir = dest.parent().unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut tmp, &session)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dest).map_err(|e| e.error)?;
    Ok(session)
}
struct Device<'a> {
    uri: &'a str,
    code: &'a str,
    secret: &'a str,
    ttl: Duration,
    interval: Duration,
}
impl<'a> Device<'a> {
    fn parse(v: &'a Value) -> Result<Self> {
        let invalid = || Error::Message("Invalid Kokoro device authorization response.");
        let field = |k: &str| v[k].as_str().ok_or_else(invalid);
        let uri = field("verification_uri")?;
        let code = field("user_code")?;
        let secret = field("device_code")?;
        if uri != "https://amamiyakoko.ro/device"
            || !(1..=64).contains(&code.len())
            || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !api::opaque(secret)
        {
            return Err(invalid());
        }
        let positive = |n: Option<u64>| {
            n.filter(|n| *n > 0)
                .map(Duration::from_secs)
                .ok_or_else(invalid)
        };
        Ok(Self {
            uri,
            code,
            secret,
            ttl: positive(v["expires_in"].as_u64())?,
            interval: positive(v.get("interval").map_or(Some(5), Value::as_u64))?,
        })
    }
}
const EXPIRED: &str = "Device code expired. Run kokorobox login again.";
fn next_interval(error: Error, current: Duration) -> Result<Duration> {
    let slower = current.saturating_add(Duration::from_secs(5));
    match error {
        Error::Timeout => Ok(current.saturating_mul(2)),
        Error::Http {
            status: 429, retry, ..
        } => Ok(slower.max(Duration::from_secs(retry))),
        Error::Http {
            status: 400, code, ..
        } => match code.as_str() {
            "authorization_pending" => Ok(current),
            "slow_down" => Ok(slower),
            "expired_token" => Err(Error::Message(EXPIRED)),
            "access_denied" => Err(Error::Message("Device login was denied.")),
            _ => Err(Error::Message(
                "Device login could not be completed. Run kokorobox login again.",
            )),
        },
        other => Err(other),
    }
}
pub fn login(l: &Locale) -> Result<()> {
    if Path::new(ROOT).join(SESSION).try_exists()? {
        return Err(Error::Message(
            "A session is already saved. Use whoami to check it or logout to switch accounts.",
        ));
    }
    let body = Body::Form(&[("client_id", CLIENT)]);
    let bytes = match api::request(
        &format!("{API}/auth/device"),
        &body,
        None,
        Format::Json,
        TIMEOUT,
    ) {
        Err(Error::Http {
            status: 404 | 405, ..
        }) => {
            return Err(Error::Message(
                "This Kokoro server does not offer device login yet.",
            ))
        }
        other => other?,
    };
    let doc = api::object(&bytes)?;
    let d = Device::parse(&doc)?;
    let start = Instant::now();
    println!(
        "{}",
        l.tr("Open {uri} on your phone or computer and confirm code {code}.")
            .replace("{uri}", d.uri)
            .replace("{code}", d.code)
    );
    io::stdout().flush()?;
    let mut interval = d.interval;
    let fields = [
        ("client_id", CLIENT),
        ("grant_type", DEVICE_GRANT),
        ("device_code", d.secret),
    ];
    loop {
        thread::sleep(interval.min(d.ttl.saturating_sub(start.elapsed())));
        let left = d.ttl.saturating_sub(start.elapsed());
        if left < Duration::from_millis(1) {
            return Err(Error::Message(EXPIRED));
        }
        match api::request(
            &format!("{API}/auth/token"),
            &Body::Form(&fields),
            None,
            Format::Json,
            left.min(TIMEOUT),
        ) {
            Ok(bytes) => {
                save_tokens(&bytes)?;
                return Ok(());
            }
            Err(e) => interval = next_interval(e, interval)?,
        }
    }
}
fn refresh(session: &Value) -> Result<Value> {
    if session["refresh_expires"].as_f64().ok_or(Error::Local)? > now()? {
        let data =
            json!({"grant_type":"refresh_token","refresh_token":token(session,"refresh_token")?});
        match api::request(
            &format!("{API}/auth/token"),
            &Body::Json(&data),
            None,
            Format::Json,
            TIMEOUT,
        ) {
            Ok(bytes) => return save_tokens(&bytes),
            Err(Error::Http { status: 401, .. }) => (),
            Err(e) => return Err(e),
        }
    }
    fs::remove_file(Path::new(ROOT).join(SESSION))?;
    Err(Error::Message(
        "Kokoro session expired. Sign in again with kokorobox login.",
    ))
}
pub fn authorized(url: &str, body: &Body, format: Format) -> Result<Vec<u8>> {
    let path = Path::new(ROOT).join(SESSION);
    if !path.try_exists()? {
        return Err(Error::Message("Sign in with kokorobox login first."));
    }
    let mut session: Value = serde_json::from_reader(fs::File::open(path)?)?;
    if session["access_expires"].as_f64().ok_or(Error::Local)? <= now()? + 15.0 {
        session = refresh(&session)?;
    }
    let send =
        |s: &Value| api::request(url, body, Some(token(s, "access_token")?), format, TIMEOUT);
    match send(&session) {
        Err(Error::Http { status: 401, .. }) => send(&refresh(&session)?),
        result => result,
    }
}
pub fn account() -> Result<Value> {
    let user = api::object(&authorized(&format!("{API}/me"), &Body::Get, Format::Json)?)?;
    if !user["osu_id"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::Message("Invalid Kokoro account response."));
    }
    Ok(user)
}
pub fn logout() -> Result<()> {
    let session = Path::new(ROOT).join(SESSION);
    if session.try_exists()? {
        let result = authorized(
            &format!("{API}/auth/revoke"),
            &Body::Json(&json!({})),
            Format::Empty,
        );
        if session.try_exists()? {
            fs::remove_file(session)?;
            result.map_err(|_| {
                Error::Message("Signed out locally. Server revocation could not be confirmed.")
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_challenge_rejects_untrusted_verification_uri() {
        let mut v = json!({"device_code":"private","user_code":"ABCD-EFGH","expires_in":60,"verification_uri":"https://amamiyakoko.ro/device"});
        assert_eq!(Device::parse(&v).unwrap().interval.as_secs(), 5);
        v["verification_uri"] = json!("https://example.invalid/device");
        assert!(Device::parse(&v).is_err());
    }
    #[test]
    fn polling_backoff_and_terminal_errors() {
        fn http(status: u32, code: &str, retry: u64) -> Error {
            Error::Http {
                status,
                code: code.into(),
                retry,
            }
        }
        let mut interval = Duration::from_secs(2);
        for (error, wait) in [
            (http(400, "slow_down", 0), 7),
            (http(400, "authorization_pending", 0), 7),
            (Error::Timeout, 14),
            (http(429, "", 30), 30),
        ] {
            interval = next_interval(error, interval).unwrap();
            assert_eq!(interval.as_secs(), wait);
        }
        assert!(matches!(
            next_interval(http(400, "access_denied", 0), interval),
            Err(Error::Message("Device login was denied."))
        ));
        assert!(matches!(
            next_interval(http(400, "expired_token", 0), interval),
            Err(Error::Message(EXPIRED))
        ));
    }
}
