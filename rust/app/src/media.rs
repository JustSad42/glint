//! Now playing and media keys, through Windows' media session API (the same one the volume
//! flyout uses), so it works with Spotify, browsers, media players...
use serde_json::{json, Value};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};

fn session() -> Option<Session> {
    Manager::RequestAsync().ok()?.get().ok()?.GetCurrentSession().ok()
}

pub struct Now {
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
}

pub fn now() -> Option<Now> {
    let s = session()?;
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().ok()?.to_string();
    if title.is_empty() {
        return None;
    }
    let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
    let playing = s.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok()) == Some(Status::Playing);
    let app = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
    Some(Now { title, artist, app: pretty_app(&app), playing })
}

fn pretty_app(aumid: &str) -> String {
    let base = aumid.rsplit(['\\', '!']).next().unwrap_or(aumid);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    let base = base.split('.').next_back().filter(|s| !s.is_empty()).unwrap_or(base);
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Cover art bytes (JPEG or PNG) of what's playing.
pub fn cover() -> Option<Vec<u8>> {
    let s = session()?;
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()?.min(8 << 20) as u32;
    if size == 0 {
        return None;
    }
    let buf = Buffer::Create(size).ok()?;
    let filled = stream.ReadAsync(&buf, size, InputStreamOptions::None).ok()?.get().ok()?;
    let n = filled.Length().ok()? as usize;
    let reader = DataReader::FromBuffer(&filled).ok()?;
    let mut out = vec![0u8; n];
    reader.ReadBytes(&mut out).ok()?;
    Some(out)
}

pub fn control(cmd: &str) -> Result<String, String> {
    let s = session().ok_or("Nothing is playing")?;
    let ok = match cmd {
        "toggle" => s.TryTogglePlayPauseAsync(),
        "next" => s.TrySkipNextAsync(),
        "prev" => s.TrySkipPreviousAsync(),
        "pause" => s.TryPauseAsync(),
        "play" => s.TryPlayAsync(),
        _ => return Err("Unknown media command".into()),
    }
    .map_err(|e| e.to_string())?
    .get()
    .unwrap_or(false);
    if !ok {
        return Err("The player didn't accept that".into());
    }
    Ok(match cmd {
        "next" => "Next track".into(),
        "prev" => "Previous track".into(),
        "pause" => "Paused".into(),
        "play" => "Playing".into(),
        _ => "Play/pause".into(),
    })
}

const WORDS: &[&str] = &["play", "pause", "next", "previous", "prev", "skip", "music", "media", "song", "track", "now playing", "playing"];

/// The now-playing card plus play/next/previous rows. `always`: show even without a media word.
pub fn rows(q: &str, always: bool) -> Vec<Value> {
    let ql = q.trim().to_lowercase();
    let wanted = always || (!ql.is_empty() && WORDS.iter().any(|w| w.starts_with(&ql) && ql.len() >= 3 || ql == *w));
    if !wanted {
        return vec![];
    }
    let Some(n) = now() else { return vec![] };
    let stamp = crate::history::now();
    let sub = [n.artist.as_str(), n.app.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
    let card = json!({
        "key": "media:now", "kind": "media", "title": n.title, "subtitle": if sub.is_empty() { "Now playing".to_string() } else { sub },
        "icon": format!("{}/media/{stamp}", crate::icons::BASE), "glyph": if n.playing { "pause" } else { "play" }, "op": "toggle",
        "playing": n.playing, "score": 4000,
    });
    if always {
        return vec![card];
    }
    let row = |op: &str, title: &str, glyph: &str, score: i32| {
        json!({"key": format!("media:{op}"), "kind": "media", "title": title, "subtitle": format!("{} · {}", n.title, n.app), "glyph": glyph, "op": op, "score": score})
    };
    let mut out = vec![card];
    if ql.starts_with("next") || ql.starts_with("skip") {
        out.insert(0, row("next", "Next track", "next", 4100));
    } else if ql.starts_with("prev") {
        out.insert(0, row("prev", "Previous track", "prev", 4100));
    } else {
        out.push(row("next", "Next track", "next", 3900));
        out.push(row("prev", "Previous track", "prev", 3800));
    }
    out
}
