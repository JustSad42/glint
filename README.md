# Glint

A fast launcher and search for Windows, written in Rust. Tap the **Windows key** and type: apps,
every file and folder on your drives, the words inside your documents, open windows, Windows
settings, a calculator and more, all in one box.

## Why it's faster than Windows Search

- **Every file name is already in memory.** A small background service reads each NTFS drive's
  file table directly (the way Everything does) and follows the change journal, so new, renamed
  and deleted files show up within a second. Searching a million files takes a few milliseconds.
- **No web results, ads or news.** Nothing you type leaves your PC. A "search the web" row is
  offered at the bottom, and only opens your browser if you pick it.
- **It learns.** What you open ranks higher next time, and the item you chose after typing
  "ch" comes first the next time you type "ch".

## What it does

| Type | Gets you |
|---|---|
| `code`, `vsc` | Apps, by name, word start, initials or exe name |
| `budget 2024` | Files and folders on every fixed drive. Words can match parent folders too |
| `*.pdf`, `ext:docx,xlsx`, `folder:` | Filter by type or kind |
| words inside a document | File contents: text and code, Word, Excel, PowerPoint, OpenDocument, EPUB, PDF |
| a window title | Switch to an open window, or close it |
| `wifi`, `sound`, `lock`, `restart` | Windows Settings pages and system actions |
| `2^10/4`, `15% of 80`, `10 km to mi`, `255 to hex` | Calculator and unit converter |
| `#7c6cff`, `rgb(…)` | Color conversions |
| `C:\`, `~\Downloads`, `%appdata%` | Browse folders, with Tab to complete |
| `>ipconfig /all` | Run a command in PowerShell (Shift+Enter: as admin) |
| `kill chrome` | End a process |
| `?anything` | Web search with your chosen engine |

**Keys:** Enter opens · Ctrl+Enter shows in folder · Shift+Enter runs as admin · → or Ctrl+K for all
actions (copy path, copy file, open with, terminal here, VS Code, properties, recycle) · Tab switches
filter · Ctrl+1…9 opens a result · ↑ on an empty box recalls past searches · Ctrl+P toggles the
preview pane.

**Win+E** can open [File Pilot](https://filepilot.tech) instead of File Explorer.

## How it's built

- `rust/core`: the file-name index (packed in-memory table, parallel ranked search), NTFS
  MFT/USN journal reader, and the named-pipe protocol.
- `rust/indexer`: `glint-indexer.exe`, a Windows service running as SYSTEM (reading a drive's file
  table needs admin rights). It answers searches over a local pipe that only the signed-in user,
  admins and SYSTEM may open.
- `rust/app`: `Glint.exe`, the Tauri 2 launcher, running as you. Apps come from the shell's
  AppsFolder, icons and thumbnails from the shell image factory, content search from a tantivy
  index kept up to date by a file watcher at background priority. A low-level keyboard hook
  turns a lone Windows-key tap into "open Glint" without breaking Win+ shortcuts.

## Build

Needs Rust and [Inno Setup 6](https://jrsoftware.org/isinfo.php).

```powershell
.\release.ps1            # tests, release build, installer in dist\
```

## Privacy

Everything stays on your PC. The only network request is the daily update check against this
repo's GitHub releases (it can be turned off). Content search never reads online-only OneDrive
files or files that typically hold secrets (`.env`, keys, password databases).

## License

MIT
