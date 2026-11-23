//! Reads an NTFS volume's file list straight from its USN change journal (needs admin rights).
//!
//! `enumerate` walks every file record in the MFT (a few seconds for a million files), and
//! `read_changes` then follows the journal so creates, deletes and renames show up within a second.
use crate::index::RawRec;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetFileInformationByHandle, GetLogicalDrives, GetVolumeInformationW, BY_HANDLE_FILE_INFORMATION};
use windows::Win32::System::IO::DeviceIoControl;

const FSCTL_ENUM_USN_DATA: u32 = 0x000900b3;
const FSCTL_READ_USN_JOURNAL: u32 = 0x000900bb;
const FSCTL_CREATE_USN_JOURNAL: u32 = 0x000900e7;
const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900f4;

const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_JOURNAL_DELETE_IN_PROGRESS: u32 = 1178;
const ERROR_JOURNAL_NOT_ACTIVE: u32 = 1179;
const ERROR_JOURNAL_ENTRY_DELETED: u32 = 1181;

const DRIVE_FIXED: u32 = 3;

pub const REASON_FILE_CREATE: u32 = 0x0000_0100;
pub const REASON_FILE_DELETE: u32 = 0x0000_0200;
pub const REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
pub const REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
/// contents written, grown or truncated, or timestamps changed: size or date may differ
pub const REASON_DATA: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004 | 0x0000_8000;
const WATCH: u32 = REASON_FILE_CREATE | REASON_FILE_DELETE | REASON_RENAME_NEW_NAME | REASON_RENAME_OLD_NAME | REASON_DATA;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn is_win32(e: &windows::core::Error, code: u32) -> bool {
    e.code() == HRESULT::from_win32(code)
}

/// Drive letters of fixed NTFS volumes.
pub fn fixed_ntfs_volumes() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let mut fs = [0u16; 32];
        if unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), None, None, None, None, Some(&mut fs)) }.is_ok() {
            let end = fs.iter().position(|&c| c == 0).unwrap_or(fs.len());
            if String::from_utf16_lossy(&fs[..end]).eq_ignore_ascii_case("NTFS") {
                out.push(letter);
            }
        }
    }
    out
}

/// One change from the journal.
#[derive(Clone, Debug)]
pub struct Change {
    pub rec: RawRec,
    pub reason: u32,
}

pub enum ReadResult {
    /// zero or more changes were delivered
    Ok,
    /// the journal was reset or wrapped past us: the whole volume must be scanned again
    Reset,
}

pub struct Volume {
    pub letter: char,
    file: File,
    pub journal_id: u64,
    pub next_usn: i64,
}

impl Volume {
    pub fn open(letter: char) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2 | 0x4)
            .open(format!(r"\\.\{letter}:"))
            .map_err(|e| format!("Couldn't open drive {letter}: {e} (the indexer needs admin rights)"))?;
        let mut v = Volume { letter, file, journal_id: 0, next_usn: 0 };
        v.query_journal()?;
        Ok(v)
    }

    fn h(&self) -> HANDLE {
        HANDLE(self.file.as_raw_handle())
    }

    /// The file reference number of the volume's root folder.
    pub fn root_frn(&self) -> Result<u64, String> {
        let root = OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2 | 0x4)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: needed to open a folder
            .open(format!("{}:\\", self.letter))
            .map_err(|e| format!("Couldn't open {}:\\ : {e}", self.letter))?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(root.as_raw_handle()), &mut info) }.map_err(|e| e.to_string())?;
        Ok(((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64)
    }

    /// Read the journal's id and current position, creating the journal if the volume has none.
    pub fn query_journal(&mut self) -> Result<(), String> {
        for attempt in 0..2 {
            let mut out = [0u8; 80];
            let mut got = 0u32;
            let r = unsafe {
                DeviceIoControl(self.h(), FSCTL_QUERY_USN_JOURNAL, None, 0, Some(out.as_mut_ptr() as _), out.len() as u32, Some(&mut got), None)
            };
            match r {
                Ok(()) => {
                    self.journal_id = u64::from_le_bytes(out[0..8].try_into().unwrap());
                    self.next_usn = i64::from_le_bytes(out[16..24].try_into().unwrap());
                    return Ok(());
                }
                Err(e) if attempt == 0 && (is_win32(&e, ERROR_JOURNAL_NOT_ACTIVE) || is_win32(&e, ERROR_JOURNAL_DELETE_IN_PROGRESS)) => {
                    // 32 MB journal, grown 4 MB at a time: the Windows default
                    let mut create = [0u8; 16];
                    create[0..8].copy_from_slice(&(32u64 << 20).to_le_bytes());
                    create[8..16].copy_from_slice(&(4u64 << 20).to_le_bytes());
                    unsafe {
                        DeviceIoControl(self.h(), FSCTL_CREATE_USN_JOURNAL, Some(create.as_ptr() as _), 16, None, 0, Some(&mut got), None)
                    }
                    .map_err(|e| format!("Couldn't turn on the change journal for {}: {e}", self.letter))?;
                }
                Err(e) => return Err(format!("Couldn't read the change journal for {}: {e}", self.letter)),
            }
        }
        Err("change journal unavailable".into())
    }

    /// Every file and folder on the volume.
    pub fn enumerate(&mut self, mut f: impl FnMut(RawRec)) -> Result<(), String> {
        self.query_journal()?;
        let mut buf = vec![0u8; 1 << 20];
        let mut start: u64 = 0;
        loop {
            // MFT_ENUM_DATA_V0: StartFileReferenceNumber, LowUsn, HighUsn
            let mut input = [0u8; 24];
            input[0..8].copy_from_slice(&start.to_le_bytes());
            input[8..16].copy_from_slice(&0i64.to_le_bytes());
            input[16..24].copy_from_slice(&self.next_usn.to_le_bytes());
            let mut got = 0u32;
            let r = unsafe {
                DeviceIoControl(
                    self.h(),
                    FSCTL_ENUM_USN_DATA,
                    Some(input.as_ptr() as _),
                    input.len() as u32,
                    Some(buf.as_mut_ptr() as _),
                    buf.len() as u32,
                    Some(&mut got),
                    None,
                )
            };
            match r {
                Ok(()) => {}
                Err(e) if is_win32(&e, ERROR_HANDLE_EOF) => return Ok(()),
                Err(e) => return Err(format!("Reading drive {} failed: {e}", self.letter)),
            }
            let got = got as usize;
            if got < 8 {
                return Ok(());
            }
            start = u64::from_le_bytes(buf[0..8].try_into().unwrap());
            parse_records(&buf[8..got], |rec, _| f(rec));
        }
    }

    /// Changes since the last call (or since `enumerate`). Doesn't wait for new ones.
    pub fn read_changes(&mut self, mut f: impl FnMut(Change)) -> Result<ReadResult, String> {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            // READ_USN_JOURNAL_DATA_V0
            let mut input = [0u8; 40];
            input[0..8].copy_from_slice(&self.next_usn.to_le_bytes());
            input[8..12].copy_from_slice(&WATCH.to_le_bytes());
            input[12..16].copy_from_slice(&0u32.to_le_bytes()); // every change, not only on close
            input[16..24].copy_from_slice(&0u64.to_le_bytes()); // timeout
            input[24..32].copy_from_slice(&0u64.to_le_bytes()); // don't wait for data
            input[32..40].copy_from_slice(&self.journal_id.to_le_bytes());
            let mut got = 0u32;
            let r = unsafe {
                DeviceIoControl(
                    self.h(),
                    FSCTL_READ_USN_JOURNAL,
                    Some(input.as_ptr() as _),
                    input.len() as u32,
                    Some(buf.as_mut_ptr() as _),
                    buf.len() as u32,
                    Some(&mut got),
                    None,
                )
            };
            match r {
                Ok(()) => {}
                Err(e) if is_win32(&e, ERROR_JOURNAL_ENTRY_DELETED) || is_win32(&e, ERROR_JOURNAL_NOT_ACTIVE) || is_win32(&e, ERROR_JOURNAL_DELETE_IN_PROGRESS) => {
                    return Ok(ReadResult::Reset)
                }
                Err(e) if e.code() == HRESULT::from_win32(87) => return Ok(ReadResult::Reset), // journal id changed
                Err(e) => return Err(format!("Reading changes on {} failed: {e}", self.letter)),
            }
            let got = got as usize;
            if got < 8 {
                return Ok(ReadResult::Ok);
            }
            let next = i64::from_le_bytes(buf[0..8].try_into().unwrap());
            let mut any = false;
            parse_records(&buf[8..got], |rec, reason| {
                any = true;
                f(Change { rec, reason })
            });
            let advanced = next != self.next_usn;
            self.next_usn = next;
            if !any || !advanced {
                return Ok(ReadResult::Ok);
            }
        }
    }
}

/// Walk USN_RECORD_V2 records packed in `buf`.
fn parse_records(buf: &[u8], mut f: impl FnMut(RawRec, u32)) {
    let mut off = 0usize;
    while off + 60 <= buf.len() {
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        if len < 60 || off + len > buf.len() {
            break;
        }
        let major = u16::from_le_bytes(buf[off + 4..off + 6].try_into().unwrap());
        if major == 2 {
            let r = &buf[off..off + len];
            let frn = u64::from_le_bytes(r[8..16].try_into().unwrap());
            let parent = u64::from_le_bytes(r[16..24].try_into().unwrap());
            let reason = u32::from_le_bytes(r[40..44].try_into().unwrap());
            let attrs = u32::from_le_bytes(r[52..56].try_into().unwrap());
            let name_len = u16::from_le_bytes(r[56..58].try_into().unwrap()) as usize;
            let name_off = u16::from_le_bytes(r[58..60].try_into().unwrap()) as usize;
            if name_off + name_len <= len {
                let units: Vec<u16> = r[name_off..name_off + name_len].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                f(RawRec { frn, parent, name: String::from_utf16_lossy(&units), attrs }, reason);
            }
        }
        off += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_record() {
        let name: Vec<u8> = "héllo.txt".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let len = (60 + name.len() + 7) & !7;
        let mut r = vec![0u8; len];
        r[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        r[4..6].copy_from_slice(&2u16.to_le_bytes());
        r[8..16].copy_from_slice(&42u64.to_le_bytes());
        r[16..24].copy_from_slice(&5u64.to_le_bytes());
        r[40..44].copy_from_slice(&REASON_FILE_CREATE.to_le_bytes());
        r[52..56].copy_from_slice(&0x20u32.to_le_bytes());
        r[56..58].copy_from_slice(&(name.len() as u16).to_le_bytes());
        r[58..60].copy_from_slice(&60u16.to_le_bytes());
        r[60..60 + name.len()].copy_from_slice(&name);
        let mut buf = r.clone();
        buf.extend_from_slice(&r);
        let mut seen = vec![];
        parse_records(&buf, |rec, reason| seen.push((rec, reason)));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0.name, "héllo.txt");
        assert_eq!(seen[0].0.frn, 42);
        assert_eq!(seen[0].1, REASON_FILE_CREATE);
    }
}
