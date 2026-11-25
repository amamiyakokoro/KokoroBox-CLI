use crate::api::{self, Body, Error, Format, Result, API};
use crate::{auth, Locale, ROOT};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
    process::Command,
    time::Duration,
};

fn lock(store: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(store.join("lock"))?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}
fn read_object(path: &Path) -> Result<Value> {
    match File::open(path) {
        Ok(f) => {
            let data: Value = serde_json::from_reader(f)?;
            if !data.is_object() {
                return Err(Error::Local);
            }
            Ok(data)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn backend(l: &Locale, script: &str) -> Command {
    let mut cmd = Command::new("python3");
    cmd.current_dir(ROOT)
        .env("KOKOROBOX_UI_LANG", l.code)
        .arg(script);
    cmd
}
struct Edit {
    owner: String,
    before: Value,
    after: Value,
}
fn edit(l: &Locale, store: &Path) -> Result<Option<Edit>> {
    let (owner, before, options) = {
        let _lock = lock(store)?;
        let user = auth::account()?;
        let owner = user["osu_id"].as_str().ok_or(Error::Local)?.to_owned();
        let saved = read_object(&store.join("current/selections.json"))?;
        let before = saved.get(&owner).cloned().unwrap_or_else(|| json!({}));
        let options = api::object(&auth::authorized(
            &format!("{API}/subscription/options"),
            &Body::Get,
            Format::Json,
        )?)?;
        (owner, before, options)
    };
    // The terminal stays attached to the editor, while unattended sync is free to run.
    let temp = tempfile::tempdir_in(store)?;
    let input = temp.path().join("options");
    let output = temp.path().join("selection");
    write_private(
        &input,
        &serde_json::to_vec(&json!({"options":options,"saved":before}))?,
    )?;
    if !backend(l, "kokoro_selection.py")
        .arg(&input)
        .arg(&output)
        .status()?
        .success()
    {
        return Err(Error::Reported);
    }
    if !output.try_exists()? {
        return Ok(None);
    }
    Ok(Some(Edit {
        owner,
        before,
        after: read_object(&output)?,
    }))
}

fn query(url: &str) -> form_urlencoded::Parse<'_> {
    form_urlencoded::parse(url.split_once('?').map_or("", |(_, q)| q).as_bytes())
}
fn external(url: &str) -> Result<Vec<u8>> {
    let mut uuid = query(url).filter(|(k, _)| k == "uuid");
    let mut client = query(url).filter(|(k, _)| k == "client");
    if !api::endpoint(url, "/api/config")
        || !uuid.next().is_some_and(|(_, v)| !v.is_empty())
        || uuid.next().is_some()
        || client.next().is_some_and(|(_, v)| v != "meta")
        || client.next().is_some()
    {
        return Err(Error::Message(
            "Use the Kokoro external Mihomo subscription URL.",
        ));
    }
    api::request(
        url,
        &Body::Get,
        None,
        Format::Profile,
        Duration::from_secs(30),
    )
}
fn account_profile(settings: Option<&Value>) -> Result<(Vec<u8>, String)> {
    let mut params = settings.cloned().unwrap_or_else(|| json!({}));
    params
        .as_object_mut()
        .ok_or(Error::Local)?
        .insert("format".into(), json!("mihomo"));
    let resolved = match auth::authorized(
        &format!("{API}/subscription/resolve"),
        &Body::Json(&params),
        Format::Json,
    ) {
        Err(Error::Http { status: 422, .. }) if settings.is_some() => {
            return Err(Error::Message(
                "Subscription selection is no longer available. Run kokorobox select.",
            ))
        }
        other => api::object(&other?)?,
    };
    let url = resolved["authenticated_config_url"].as_str().unwrap_or("");
    if resolved["format"] != "mihomo"
        || !api::endpoint(url, "/api/app/subscription/config")
        || !api::yaml_type(resolved["content_type"].as_str().unwrap_or(""))
    {
        return Err(Error::Message(
            "Kokoro returned an incompatible profile or download address.",
        ));
    }
    Ok((
        auth::authorized(url, &Body::Get, Format::Profile)?,
        url.to_owned(),
    ))
}

fn update(l: &Locale, store: &Path, imported: Option<String>, edit: Option<Edit>) -> Result<bool> {
    let profile = Path::new(ROOT).join("yamls/config.yaml");
    let active =
        fs::read_link(&profile).is_ok_and(|v| v == Path::new(".kokoro/current/config.yaml"));
    let current = store.join("current");
    let old_info = if active {
        read_object(&current.join("active.json"))?
    } else {
        json!({})
    };
    let old_url = if active && current.join("url").try_exists()? {
        Some(fs::read_to_string(current.join("url"))?.trim().to_owned())
    } else {
        None
    };
    let mut url = imported;
    if url.is_none() && edit.is_none() {
        url.clone_from(&old_url);
        if url.is_none() && old_info["source"] == "external" {
            return Err(Error::Message(
                "Use the Kokoro external Mihomo subscription URL.",
            ));
        }
    }
    let mut saved = read_object(&current.join("selections.json"))?;
    let mut changed_selection = false;
    let (bytes, resolved, mut info) = if let Some(link) = &url {
        (external(link)?, link.clone(), json!({"source":"external"}))
    } else {
        if !Path::new(ROOT).join(auth::SESSION).try_exists()? {
            return Err(Error::Message(if active {
                "Sign in with kokorobox login to refresh this account profile."
            } else {
                "Run kokorobox login or kokorobox subscribe first."
            }));
        }
        let user = auth::account()?;
        let owner = user["osu_id"].as_str().ok_or(Error::Local)?;
        let expected = edit
            .as_ref()
            .map(|e| e.owner.as_str())
            .or_else(|| old_info["account_id"].as_str());
        if expected.is_some_and(|id| id != owner) {
            return Err(Error::Message(
                "The signed-in account changed. Run kokorobox select again.",
            ));
        }
        if let Some(edit) = edit {
            let before = saved.get(owner).cloned().unwrap_or_else(|| json!({}));
            if before != edit.before {
                return Err(Error::Message(
                    "Subscription settings changed. Run kokorobox select again.",
                ));
            }
            changed_selection = before != edit.after;
            saved[owner] = edit.after;
        }
        let (bytes, resolved) = account_profile(saved.get(owner))?;
        (
            bytes,
            resolved,
            json!({"source":"account","account_id":owner}),
        )
    };
    let settings = info["account_id"].as_str().and_then(|id| saved.get(id));
    for key in ["plan", "protocol", "rule_source"] {
        let field = if url.is_some() && key == "rule_source" {
            "rule"
        } else {
            key
        };
        let value = query(&resolved)
            .find(|(k, v)| k == field && !v.is_empty())
            .map(|(_, v)| v.into_owned())
            .or_else(|| settings.and_then(|v| v[key].as_str()).map(str::to_owned));
        if let Some(s) = value.filter(|s| !s.chars().any(char::is_control)) {
            info[key] = json!(s);
        }
    }
    if active
        && !changed_selection
        && old_url == url
        && old_info == info
        && fs::read(&profile)? == bytes
    {
        return Ok(false);
    }
    let stage = tempfile::Builder::new()
        .prefix("profile-")
        .tempdir_in(store)?;
    write_private(&stage.path().join("config.yaml"), &bytes)?;
    write_private(
        &stage.path().join("active.json"),
        &serde_json::to_vec(&info)?,
    )?;
    if saved != json!({}) {
        write_private(
            &stage.path().join("selections.json"),
            &serde_json::to_vec(&saved)?,
        )?;
    }
    if let Some(link) = url {
        write_private(&stage.path().join("url"), link.as_bytes())?;
    }
    // The service backend validates, reloads and publishes together under its service lock.
    let result = backend(l, "runtime.py")
        .arg("apply-profile")
        .arg(stage.path())
        .status();
    if fs::read_link(&current)
        .is_ok_and(|target| Some(target.as_os_str()) == stage.path().file_name())
    {
        let _ = stage.keep();
    }
    if !result?.success() {
        return Err(Error::Reported);
    }
    Ok(true)
}

pub fn run(command: &str, l: &Locale) -> Result<()> {
    let store = Path::new(ROOT).join("yamls/.kokoro");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&store)?;
    let mut imported = None;
    let mut selected = None;
    if command == "subscribe" {
        let input = if io::stdin().is_terminal() {
            rpassword::prompt_password(l.tr("Kokoro Mihomo subscription URL: "))?
        } else {
            let mut s = String::new();
            io::stdin().read_line(&mut s)?;
            s.trim_end_matches(['\r', '\n']).to_owned()
        };
        imported = Some(input);
    } else if command == "select" {
        selected = edit(l, &store)?;
        if selected.is_none() {
            println!("{}", l.tr("Selection cancelled."));
            return Ok(());
        }
    }
    let _lock = lock(&store)?;
    let message = match command {
        "login" => {
            auth::login(l)?;
            "Signed in. Use kokorobox select to switch to an account subscription."
        }
        "logout" => {
            auth::logout()?;
            "Signed out."
        }
        "whoami" => {
            let user = auth::account()?;
            let id = user["osu_id"].as_str().ok_or(Error::Local)?;
            if let Some(name) = user["username"]
                .as_str()
                .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
            {
                println!("{name} (osu! {id})");
            } else {
                println!("osu! {id}");
            }
            return Ok(());
        }
        _ => {
            if update(l, &store, imported, selected)? {
                "Subscription updated."
            } else {
                "Profile is current."
            }
        }
    };
    println!("{}", l.tr(message));
    Ok(())
}
