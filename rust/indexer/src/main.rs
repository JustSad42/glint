//! Glint Indexer: a small Windows service that keeps the name of every file and folder on
//! your fixed NTFS drives in memory, follows changes as they happen, and answers searches
//! from the launcher over a local named pipe.
//!
//!   glint-indexer.exe              run as the service (started by Windows)
//!   glint-indexer.exe --install    register and start the service (needs admin)
//!   glint-indexer.exe --uninstall  stop and remove the service (needs admin)
//!   glint-indexer.exe --console    run in this console instead (needs admin; for debugging)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use glint_core::index::{Builder, VolumeIndex};
use glint_core::ntfs::{self, ReadResult, Volume, REASON_DATA, REASON_FILE_CREATE, REASON_FILE_DELETE, REASON_RENAME_NEW_NAME};
use glint_core::proto::{ChildrenReply, Hit, Request, SearchReply, StatusReply, VolumeStatus};
use glint_core::{pipe, PIPE_NAME, SERVICE_NAME};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("Glint")
}

static LOG_LOCK: Mutex<()> = Mutex::new(());

fn log(msg: &str) {
    let _g = LOG_LOCK.lock();
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("indexer.log");
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("indexer.old.log"));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
    if cfg!(debug_assertions) || std::env::args().any(|a| a == "--console") {
        eprintln!("{msg}");
    }
}

struct Vol {
    letter: char,
    index: RwLock<VolumeIndex>,
    status: Mutex<VolumeStatus>,
    /// bumped on every full scan, so an older size/date pass stops
    scan_id: Mutex<u64>,
}

struct Engine {
    vols: Vec<Arc<Vol>>,
}

impl Engine {
    fn start(stop: Arc<AtomicBool>) -> Arc<Engine> {
        let letters = ntfs::fixed_ntfs_volumes();
        log(&format!("Glint Indexer {VERSION}: drives {letters:?}"));
        let vols: Vec<Arc<Vol>> = letters
            .into_iter()
            .map(|l| {
                Arc::new(Vol {
                    letter: l,
                    index: RwLock::new(VolumeIndex::new(l, 0)),
                    status: Mutex::new(VolumeStatus { letter: l, ..Default::default() }),
                    scan_id: Mutex::new(0),
                })
            })
            .collect();
        for v in &vols {
            let v = v.clone();
            let stop = stop.clone();
            std::thread::Builder::new().name(format!("vol-{}", v.letter)).spawn(move || watch_volume(v, stop)).ok();
        }
        Arc::new(Engine { vols })
    }

    fn handle(&self, line: &str) -> String {
        let req: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => return serde_json::json!({"error": format!("bad request: {e}")}).to_string(),
        };
        match req {
            Request::Status => {
                let volumes: Vec<VolumeStatus> = self.vols.iter().map(|v| v.status.lock().clone()).collect();
                let total = volumes.iter().map(|v| v.entries).sum();
                serde_json::to_string(&StatusReply { version: VERSION.into(), volumes, total }).unwrap()
            }
            Request::Search { query } => {
                let t = Instant::now();
                let mut hits: Vec<Hit> = Vec::new();
                if !query.is_empty() {
                    for v in &self.vols {
                        let idx = v.index.read();
                        for s in idx.search(&query) {
                            let (mtime, size) = idx.meta(s.idx);
                            let dir = idx.is_dir(s.idx);
                            hits.push(Hit {
                                path: idx.path(s.idx),
                                dir,
                                score: s.score,
                                mtime: (mtime != 0).then_some(mtime),
                                size: (mtime != 0 && (!dir || size > 0)).then_some(size),
                            });
                        }
                    }
                    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.len().cmp(&b.path.len())));
                    hits.truncate(query.limit.max(1));
                }
                serde_json::to_string(&SearchReply { hits, micros: t.elapsed().as_micros() as u64, error: None }).unwrap()
            }
            Request::Children { path, limit } => {
                let letter = path.chars().next().unwrap_or('C').to_ascii_uppercase();
                let mut reply = ChildrenReply::default();
                match self.vols.iter().find(|v| v.letter.to_ascii_uppercase() == letter) {
                    None => reply.error = Some(format!("{letter}: isn't indexed")),
                    Some(v) => {
                        let ready = v.status.lock().meta_ready;
                        let idx = v.index.read();
                        match idx.find_dir(&path) {
                            None => reply.error = Some("Folder not found".into()),
                            Some(d) => {
                                let (total, kids) = idx.children_by_size(d, if limit == 0 { 0 } else { limit.min(500) });
                                reply.total = ready.then_some(total);
                                reply.items = kids
                                    .into_iter()
                                    .map(|(i, s)| {
                                        let (m, _) = idx.meta(i);
                                        Hit { path: idx.path(i), dir: idx.is_dir(i), score: 0, mtime: (m != 0).then_some(m), size: ready.then_some(s) }
                                    })
                                    .collect();
                            }
                        }
                    }
                }
                serde_json::to_string(&reply).unwrap()
            }
        }
    }
}

/// Scan one drive, then follow its change journal until told to stop.
fn watch_volume(v: Arc<Vol>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match scan_and_follow(&v, &stop) {
            Ok(()) => {}
            Err(e) => {
                log(&format!("{}: {e}", v.letter));
                v.status.lock().error = Some(e);
                for _ in 0..60 {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
}

fn mtime_of(m: &std::fs::Metadata) -> u32 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as u32).unwrap_or(1)
}

/// Fill in every entry's size and modified date, one directory listing at a time, at background
/// priority. Directory listings carry this for all children at once, so this is far cheaper
/// than asking about each file.
fn fill_meta(v: Arc<Vol>, stop: Arc<AtomicBool>, scan_id: u64) {
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let t = Instant::now();
    let (generation, listings) = {
        let idx = v.index.read();
        (idx.generation, idx.dir_listings())
    };
    let mut filled = 0usize;
    for chunk in listings.chunks(200) {
        if stop.load(Ordering::SeqCst) || *v.scan_id.lock() != scan_id {
            return;
        }
        let mut ups: Vec<(u32, u32, u64)> = Vec::new();
        for (dir, kids) in chunk {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            let by_name: HashMap<&str, u32> = kids.iter().map(|(n, i)| (n.as_str(), *i)).collect();
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if let Some(&i) = by_name.get(name.as_str()) {
                    if let Ok(m) = e.metadata() {
                        ups.push((i, mtime_of(&m), if m.is_dir() { 0 } else { m.len() }));
                    }
                }
            }
        }
        let mut idx = v.index.write();
        if idx.generation != generation {
            return; // renumbered under us; the next scan fills again
        }
        filled += ups.len();
        for (i, m, s) in ups {
            idx.set_meta(i, m, s);
        }
    }
    let t2 = Instant::now();
    v.index.write().compute_dir_sizes();
    v.status.lock().meta_ready = true;
    log(&format!("{}: sizes and dates for {} entries in {} ms (folder totals {} ms)", v.letter, filled, t.elapsed().as_millis(), t2.elapsed().as_millis()));
}

/// Re-read size and date for entries that just changed.
fn refresh_meta(v: &Vol, frns: &HashSet<u64>) {
    let targets: Vec<(u32, String, u64)> = {
        let idx = v.index.read();
        frns.iter().filter_map(|f| idx.index_of(*f).map(|i| (i, idx.path(i), idx.generation))).collect()
    };
    let ups: Vec<(u32, u32, u64, u64)> = targets
        .into_iter()
        .filter_map(|(i, p, g)| std::fs::symlink_metadata(&p).ok().map(|m| (i, mtime_of(&m), if m.is_dir() { 0 } else { m.len() }, g)))
        .collect();
    let mut idx = v.index.write();
    for (i, m, s, g) in ups {
        if g == idx.generation {
            idx.set_meta(i, m, s);
        }
    }
}

fn scan_and_follow(v: &Arc<Vol>, stop: &Arc<AtomicBool>) -> Result<(), String> {
    let t = Instant::now();
    let mut vol = Volume::open(v.letter)?;
    let root = vol.root_frn()?;
    let mut b = Builder::new(v.letter, root);
    vol.enumerate(|r| b.push(&r))?;
    let built = b.finish();
    let n = built.len();
    let ms = t.elapsed().as_millis() as u64;
    *v.index.write() = built;
    {
        let mut st = v.status.lock();
        st.entries = n;
        st.ready = true;
        st.error = None;
        st.scan_ms = ms;
        st.meta_ready = false;
    }
    log(&format!("{}: {} entries in {} ms", v.letter, n, ms));
    let scan_id = {
        let mut id = v.scan_id.lock();
        *id += 1;
        *id
    };
    {
        let (v2, stop2) = (v.clone(), stop.clone());
        std::thread::Builder::new().name(format!("meta-{}", v.letter)).spawn(move || fill_meta(v2, stop2, scan_id)).ok();
    }

    let mut changes = Vec::with_capacity(256);
    let mut last_compact_check = Instant::now();
    // folder totals are recomputed now and then after files change, not on every write
    let mut sizes_dirty = false;
    let mut last_sizes = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(400));
        changes.clear();
        match vol.read_changes(|c| changes.push(c))? {
            ReadResult::Reset => {
                log(&format!("{}: change journal reset, scanning again", v.letter));
                return Ok(());
            }
            ReadResult::Ok => {}
        }
        if !changes.is_empty() {
            let mut touched: HashSet<u64> = HashSet::new();
            {
                let mut idx = v.index.write();
                for c in &changes {
                    if c.reason & REASON_FILE_DELETE != 0 {
                        idx.remove(c.rec.frn);
                        touched.remove(&c.rec.frn);
                    } else {
                        if c.reason & (REASON_FILE_CREATE | REASON_RENAME_NEW_NAME) != 0 {
                            idx.upsert(c.rec.frn, c.rec.parent, &c.rec.name, c.rec.attrs);
                        }
                        if c.reason & (REASON_FILE_CREATE | REASON_RENAME_NEW_NAME | REASON_DATA) != 0 {
                            touched.insert(c.rec.frn);
                        }
                    }
                }
                v.status.lock().entries = idx.len();
            }
            if !touched.is_empty() {
                refresh_meta(v, &touched);
            }
            sizes_dirty = true;
        }
        if sizes_dirty && last_sizes.elapsed() > Duration::from_secs(60) && v.status.lock().meta_ready {
            v.index.write().compute_dir_sizes();
            sizes_dirty = false;
            last_sizes = Instant::now();
        }
        if last_compact_check.elapsed() > Duration::from_secs(600) {
            last_compact_check = Instant::now();
            // compacting renumbers entries, so wait until sizes and dates are filled in
            if v.status.lock().meta_ready && v.index.read().waste() > 0.25 {
                v.index.write().compact();
                log(&format!("{}: compacted", v.letter));
            }
        }
    }
    Ok(())
}

fn run(stop: Arc<AtomicBool>) {
    let engine = Engine::start(stop.clone());
    let e2 = engine.clone();
    let handler: Arc<dyn Fn(&str) -> String + Send + Sync> = Arc::new(move |line| e2.handle(line));
    if let Err(e) = pipe::serve(PIPE_NAME, handler, stop) {
        log(&format!("pipe server failed: {e}"));
    }
}

// ---------------------------------------------------------------------------------------------
// Windows service plumbing

mod service {
    use super::*;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch() -> Result<(), windows_service::Error> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    fn service_main(_args: Vec<OsString>) {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop2.store(true, Ordering::SeqCst);
                pipe::poke(PIPE_NAME);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else { return };
        let set = |state: ServiceState, accept: ServiceControlAccept| {
            let _ = status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::from_secs(5),
                process_id: None,
            });
        };
        set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
        run(stop);
        set(ServiceState::Stopped, ServiceControlAccept::empty());
    }
}

mod setup {
    use super::*;
    use windows_service::service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo,
        ServiceStartType, ServiceState, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    fn wait_stopped(svc: &windows_service::service::Service) {
        for _ in 0..50 {
            match svc.query_status() {
                Ok(s) if s.current_state == ServiceState::Stopped => return,
                Err(_) => return,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    pub fn install() -> Result<(), String> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
            .map_err(|e| format!("Couldn't open the service manager (run as admin): {e}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: "Glint Indexer".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None, // LocalSystem: needed to read the drives' file tables
            account_password: None,
        };
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
        let svc = match mgr.open_service(SERVICE_NAME, access) {
            Ok(svc) => {
                let _ = svc.stop();
                wait_stopped(&svc);
                svc.change_config(&info).map_err(|e| e.to_string())?;
                svc
            }
            Err(_) => mgr.create_service(&info, access).map_err(|e| format!("Couldn't create the service: {e}"))?,
        };
        let _ = svc.set_description("Keeps the names of the files on your drives in memory so Glint can find them instantly.");
        let restart = ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) };
        let _ = svc.update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(3600)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart.clone(), restart.clone(), restart]),
        });
        svc.start::<&str>(&[]).map_err(|e| format!("Couldn't start the service: {e}"))?;
        Ok(())
    }

    pub fn uninstall() -> Result<(), String> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
        let svc = match mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
            Ok(s) => s,
            Err(_) => return Ok(()), // not installed
        };
        let _ = svc.stop();
        wait_stopped(&svc);
        svc.delete().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn stop() {
        if let Ok(mgr) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
            if let Ok(svc) = mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) {
                let _ = svc.stop();
                wait_stopped(&svc);
            }
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let code = match arg.as_str() {
        "--install" => report(setup::install()),
        "--uninstall" => report(setup::uninstall()),
        "--stop" => {
            setup::stop();
            0
        }
        "--console" => {
            let stop = Arc::new(AtomicBool::new(false));
            run(stop);
            0
        }
        _ => match service::dispatch() {
            Ok(()) => 0,
            Err(e) => {
                log(&format!("Not started by the service manager ({e}). Use --install or --console."));
                1
            }
        },
    };
    std::process::exit(code);
}

fn report(r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            log(&e);
            1
        }
    }
}
