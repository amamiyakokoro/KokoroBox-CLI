//! HTTPS transport for the Kokoro API. Response bodies and credentials never enter diagnostics.
use crate::{Locale, VERSION};
use curl::easy::{Easy, List};
use serde_json::Value;
use std::{env, io, time::Duration};

pub const ORIGIN: &str = "https://amamiyakoko.ro";
pub const API: &str = "https://amamiyakoko.ro/api/app";
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Message(&'static str),
    Http {
        status: u32,
        code: String,
        retry: u64,
    },
    Timeout,
    Transport(u32),
    Local,
    Reported,
}

impl Error {
    pub fn show(&self, l: &Locale) {
        match self {
            Self::Reported => (),
            Self::Transport(code) => eprintln!(
                "{} (libcurl {code})",
                l.tr("Unable to contact Kokoro or read or save local data.")
            ),
            Self::Http { status, .. } => eprintln!(
                "{}",
                l.tr("Kokoro returned HTTP {status}.")
                    .replace("{status}", &status.to_string())
            ),
            Self::Message(key) => eprintln!("{}", l.tr(key)),
            _ => eprintln!(
                "{}",
                l.tr("Unable to contact Kokoro or read or save local data.")
            ),
        }
    }
}
impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::Local
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Local
    }
}
impl From<curl::Error> for Error {
    fn from(e: curl::Error) -> Self {
        if e.is_operation_timedout() {
            Self::Timeout
        } else {
            Self::Transport(e.code())
        }
    }
}

pub fn endpoint(url: &str, path: &str) -> bool {
    url.len() <= 8192
        && url
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'#' | b'\\'))
        && url
            .split('?')
            .next()
            .and_then(|base| base.strip_prefix(ORIGIN))
            == Some(path)
}
pub fn yaml_type(s: &str) -> bool {
    matches!(
        s.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "text/yaml" | "application/yaml" | "application/x-yaml"
    )
}
pub enum Body<'a> {
    Get,
    Json(&'a Value),
    Form(&'a [(&'a str, &'a str)]),
}
#[derive(Clone, Copy)]
pub enum Format {
    Json,
    Profile,
    Empty,
}

pub fn request(
    url: &str,
    body: &Body,
    token: Option<&str>,
    format: Format,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut easy = Easy::new();
    easy.url(url)?;
    easy.useragent(&format!("KokoroBox-Shell/{VERSION} (mihomo)"))?;
    easy.timeout(timeout)?;
    if let Some(ca) = env::var_os("SSL_CERT_FILE").filter(|v| !v.is_empty()) {
        easy.cainfo(std::path::Path::new(&ca))?;
    }
    let mut headers = List::new();
    if let Some(token) = token {
        if !opaque(token) {
            return Err(Error::Local);
        }
        headers.append(&format!("Authorization: Bearer {token}"))?;
    }
    match body {
        Body::Get => (),
        Body::Json(value) => {
            headers.append("Content-Type: application/json")?;
            easy.post_fields_copy(&serde_json::to_vec(value)?)?;
        }
        Body::Form(fields) => {
            let encoded = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields.iter().copied())
                .finish();
            headers.append("Content-Type: application/x-www-form-urlencoded")?;
            easy.post_fields_copy(encoded.as_bytes())?;
        }
    }
    easy.http_headers(headers)?;
    let limit = if matches!(format, Format::Profile) {
        16 * 1024 * 1024
    } else {
        256 * 1024
    };
    let mut bytes = Vec::new();
    let mut oversized = false;
    let mut retry = 0;
    let result = {
        let mut transfer = easy.transfer();
        transfer.header_function(|line| {
            if let Ok(line) = std::str::from_utf8(line) {
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("Retry-After") {
                        let value = value.trim();
                        if value.len() <= 9 {
                            retry = value.parse().unwrap_or(0);
                        }
                    }
                }
            }
            true
        })?;
        transfer.write_function(|chunk| {
            if bytes.len() + chunk.len() > limit {
                oversized = true;
                return Ok(0);
            }
            bytes.extend_from_slice(chunk);
            Ok(chunk.len())
        })?;
        transfer.perform()
    };
    if oversized {
        return Err(Error::Message(if matches!(format, Format::Profile) {
            "Invalid or oversized Kokoro profile."
        } else {
            "Invalid Kokoro response."
        }));
    }
    result?;
    let status = easy.response_code()?;
    if status >= 400 {
        let error = serde_json::from_slice::<Value>(&bytes).unwrap_or_default();
        return Err(Error::Http {
            status,
            code: error["error"].as_str().unwrap_or("").into(),
            retry,
        });
    }
    if status
        != if matches!(format, Format::Empty) {
            204
        } else {
            200
        }
    {
        return Err(Error::Message("Unexpected Kokoro response."));
    }
    if matches!(format, Format::Profile) {
        if !yaml_type(easy.content_type()?.unwrap_or("")) {
            return Err(Error::Message("Expected a Mihomo YAML subscription."));
        }
        if !bytes
            .split(|b| *b == b'\n')
            .any(|l| l.starts_with(b"proxies:") || l.starts_with(b"proxy-providers:"))
        {
            return Err(Error::Message("Invalid or oversized Kokoro profile."));
        }
    }
    Ok(bytes)
}
pub fn object(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or(Error::Message("Invalid Kokoro response."))
}
pub fn opaque(s: &str) -> bool {
    (1..=4096).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic())
}
