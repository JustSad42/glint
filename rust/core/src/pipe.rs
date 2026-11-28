//! The local named pipe between the launcher and the indexer: one JSON request per line,
//! one JSON reply per line. Remote clients are rejected; only SYSTEM, admins and the
//! signed-in (interactive) user may connect. Each request carries who is asking.
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{RevertToSelf, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
use windows::Win32::UI::Shell::GetUserProfileDirectoryW;
use windows::core::PWSTR;
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, ImpersonateNamedPipeClient, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const PIPE_ACCESS_DUPLEX: u32 = 3;
const ERROR_PIPE_CONNECTED: u32 = 535;
/// SYSTEM and admins: full access. Interactive users: read + write.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

/// The longest request line accepted.
const MAX_LINE: u64 = 64 * 1024;

/// Who is on the other end of the pipe.
#[derive(Clone, Debug, Default)]
pub struct Caller {
    /// Their profile folder (`None` if it couldn't be found out).
    pub profile: Option<String>,
}

/// Borrows the client's identity just long enough to ask for its profile folder. Windows allows
/// this only after something has been read from the pipe.
fn caller(pipe: HANDLE) -> Caller {
    unsafe {
        if ImpersonateNamedPipeClient(pipe).is_err() {
            return Caller::default();
        }
        let mut token = HANDLE::default();
        let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token).is_ok();
        // Never carry on as the client: if this fails the process must not continue.
        if RevertToSelf().is_err() {
            std::process::abort();
        }
        if !opened {
            return Caller::default();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let profile = GetUserProfileDirectoryW(token, Some(PWSTR(buf.as_mut_ptr())), &mut len)
            .is_ok()
            .then(|| String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap_or(0)]))
            .filter(|p| !p.is_empty());
        let _ = CloseHandle(token);
        Caller { profile }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Serve requests until `stop` is set. `handle` turns one request line into one reply line.
pub type Handler = Arc<dyn Fn(&str, &Caller) -> String + Send + Sync>;

pub fn serve(name: &str, handle: Handler, stop: Arc<AtomicBool>) -> Result<(), String> {
    let wname = wide(name);
    let sddl = wide(SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut sd, None) }.map_err(|e| e.to_string())?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let result = {
        while !stop.load(Ordering::SeqCst) {
            let h = unsafe {
                CreateNamedPipeW(
                    PCWSTR(wname.as_ptr()),
                    FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX),
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_UNLIMITED_INSTANCES,
                    1 << 16,
                    1 << 16,
                    0,
                    Some(&sa),
                )
            };
            if h.is_invalid() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            if let Err(e) = unsafe { ConnectNamedPipe(h, None) } {
                if e.code() != windows::core::HRESULT::from_win32(ERROR_PIPE_CONNECTED) {
                    unsafe { drop(File::from_raw_handle(h.0)) };
                    continue;
                }
            }
            if stop.load(Ordering::SeqCst) {
                unsafe { drop(File::from_raw_handle(h.0)) };
                break;
            }
            let file = unsafe { File::from_raw_handle(h.0) };
            let handle = handle.clone();
            std::thread::spawn(move || {
                let mut writer = match file.try_clone() {
                    Ok(w) => w,
                    Err(_) => return,
                };
                let raw = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&file));
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                let mut who: Option<Caller> = None;
                loop {
                    line.clear();
                    match reader.by_ref().take(MAX_LINE).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(n) if n as u64 == MAX_LINE && !line.ends_with('\n') => break,
                        Ok(_) => {}
                    }
                    let who = who.get_or_insert_with(|| caller(raw));
                    let mut reply = handle(line.trim_end(), who);
                    reply.push('\n');
                    if writer.write_all(reply.as_bytes()).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    result
}

/// Wake a server blocked in ConnectNamedPipe so it can notice `stop`.
pub fn poke(name: &str) {
    let _ = OpenOptions::new().read(true).write(true).open(name);
}

/// A connection to the indexer that reconnects by itself.
pub struct Client {
    name: String,
    conn: Option<(BufReader<File>, File)>,
}

impl Client {
    pub fn new(name: &str) -> Self {
        Client { name: name.to_string(), conn: None }
    }

    fn connect(&mut self) -> std::io::Result<()> {
        let f = OpenOptions::new().read(true).write(true).open(&self.name)?;
        let w = f.try_clone()?;
        self.conn = Some((BufReader::new(f), w));
        Ok(())
    }

    /// Send one request line and read the reply line.
    pub fn call(&mut self, request: &str) -> std::io::Result<String> {
        for attempt in 0..2 {
            if self.conn.is_none() {
                self.connect()?;
            }
            let (r, w) = self.conn.as_mut().unwrap();
            let sent = w.write_all(request.as_bytes()).and_then(|_| w.write_all(b"\n")).and_then(|_| w.flush());
            let mut line = String::new();
            let got = sent.and_then(|_| r.read_line(&mut line));
            match got {
                Ok(n) if n > 0 => return Ok(line),
                _ => {
                    self.conn = None;
                    if attempt == 1 {
                        return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "indexer closed the connection"));
                    }
                }
            }
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real pipe: the server learns the connecting user's profile, answers as itself, and drops
    /// a client that sends an endless line.
    #[test]
    fn the_server_knows_who_is_asking() {
        let name = format!(r"\\.\pipe\glint-test-{}", std::process::id());
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (name, stop) = (name.clone(), stop.clone());
            std::thread::spawn(move || serve(&name, Arc::new(|line, who| format!("{line}|{}", who.profile.as_deref().unwrap_or("?"))), stop))
        };
        let mut client = Client::new(&name);
        let reply = (0..100)
            .find_map(|_| {
                client.call("hi").ok().or_else(|| {
                    std::thread::sleep(Duration::from_millis(20));
                    None
                })
            })
            .expect("no reply");
        let mine = std::env::var("USERPROFILE").unwrap();
        assert_eq!(reply.trim_end().to_lowercase(), format!("hi|{}", mine.to_lowercase()));
        assert!(client.call(&"x".repeat(MAX_LINE as usize + 10)).is_err(), "an endless line is refused");
        stop.store(true, Ordering::SeqCst);
        drop(client);
        poke(&name);
        server.join().unwrap().unwrap();
    }
}
