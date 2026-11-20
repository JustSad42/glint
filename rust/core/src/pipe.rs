//! The local named pipe between the launcher and the indexer: one JSON request per line,
//! one JSON reply per line. Remote clients are rejected; only SYSTEM, admins and the
//! signed-in (interactive) user may connect.
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const PIPE_ACCESS_DUPLEX: u32 = 3;
const ERROR_PIPE_CONNECTED: u32 = 535;
/// SYSTEM and admins: full access. Interactive users: read + write.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Serve requests until `stop` is set. `handle` turns one request line into one reply line.
pub fn serve(name: &str, handle: Arc<dyn Fn(&str) -> String + Send + Sync>, stop: Arc<AtomicBool>) -> Result<(), String> {
    let wname = wide(name);
    let sddl = wide(SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut sd, None) }.map_err(|e| e.to_string())?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let result = (|| {
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
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let mut reply = handle(line.trim_end());
                    reply.push('\n');
                    if writer.write_all(reply.as_bytes()).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    })();
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
