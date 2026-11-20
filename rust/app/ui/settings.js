// Glint settings window.
const $ = (id) => document.getElementById(id);
let S = {};
let status = null;
let page = (location.hash || '#general').slice(1);

const PAGES = [
  { id: 'welcome', label: 'Welcome', icon: 'sparkle', hidden: true },
  { id: 'general', label: 'General', icon: 'gear' },
  { id: 'keyboard', label: 'Keyboard', icon: 'keyboard' },
  { id: 'search', label: 'Search', icon: 'search' },
  { id: 'content', label: 'File contents', icon: 'filetext' },
  { id: 'extras', label: 'Extras & pins', icon: 'bolt' },
  { id: 'appearance', label: 'Appearance', icon: 'palette' },
  { id: 'privacy', label: 'Privacy & data', icon: 'shield' },
  { id: 'about', label: 'About & updates', icon: 'info' },
];

const ACCENTS = ['#7c6cff', '#3b82f6', '#06b6d4', '#10b981', '#f59e0b', '#f97316', '#ef4444', '#ec4899'];

$('logo').innerHTML = glyph('glint', 16);
$('minBtn').innerHTML = glyph('minus', 14);
$('closeBtn').innerHTML = glyph('x', 14);
$('minBtn').onclick = () => T.window.getCurrentWindow().minimize();
$('closeBtn').onclick = () => T.window.getCurrentWindow().close();

let toastTimer = 0;
function toast(msg, err = false) {
  const t = $('toast');
  t.className = `toast on ${err ? 'err' : ''}`;
  t.innerHTML = `${glyph(err ? 'warn' : 'check', 15)}${esc(msg)}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('on'), 2200);
}

async function save(patch, msg) {
  Object.assign(S, patch);
  try {
    await call('settings_set', patch);
    applyTheme(S);
    if (msg) toast(msg);
  } catch (e) {
    toast(String(e), true);
  }
}

// ------------------------------------------------------------------ building blocks
const sw = (key, on) => `<label class="sw"><input type="checkbox" data-key="${key}" ${on ? 'checked' : ''}/><span></span></label>`;
const item = (icon, title, desc, ctl) => `<div class="item"><div class="ic">${glyph(icon, 18)}</div><div class="lbl"><b>${title}</b>${desc ? `<span>${desc}</span>` : ''}</div><div class="ctl">${ctl}</div></div>`;
const select = (key, val, opts) => `<select data-key="${key}">${opts.map(([v, l]) => `<option value="${v}" ${v === val ? 'selected' : ''}>${l}</option>`).join('')}</select>`;
const seg = (key, val, opts) => `<div class="seg" data-seg="${key}">${opts.map(([v, l]) => `<button data-v="${v}" class="${v === val ? 'on' : ''}">${l}</button>`).join('')}</div>`;

function bind() {
  const c = $('content');
  c.querySelectorAll('input[type=checkbox][data-key]').forEach((el) => (el.onchange = () => save({ [el.dataset.key]: el.checked }, 'Saved')));
  c.querySelectorAll('select[data-key]').forEach((el) => (el.onchange = () => save({ [el.dataset.key]: el.value }, 'Saved')));
  c.querySelectorAll('[data-seg]').forEach((g) =>
    g.querySelectorAll('button').forEach((b) => (b.onclick = () => {
      g.querySelectorAll('button').forEach((x) => x.classList.toggle('on', x === b));
      save({ [g.dataset.seg]: b.dataset.v });
    })),
  );
}

// ------------------------------------------------------------------ status cards
function indexerCard() {
  const idx = status?.indexer;
  if (idx && idx.volumes?.length) {
    const ready = idx.volumes.every((v) => v.ready);
    const errs = idx.volumes.filter((v) => v.error);
    return `<div class="stat"><div class="big-ic ${ready ? 'ok' : 'busy'}">${glyph(ready ? 'check' : 'hdd', 22)}</div><div class="grow">
      <div class="s1">${ready ? `${fmtCount(idx.total)} files and folders indexed` : 'Reading your drives…'}</div>
      <div class="s2">The indexer service reads each drive's file table directly and follows changes as they happen.${errs.length ? ` Problem: ${esc(errs[0].error)}` : ''}</div>
      <div class="pills">${idx.volumes.map((v) => `<span class="pill">${v.letter}: · ${fmtCount(v.entries)}${v.scan_ms ? ` · scanned in ${(v.scan_ms / 1000).toFixed(1)}s` : ''}</span>`).join('')}</div>
      ${ready ? '' : '<div class="progress indet"><div></div></div>'}
    </div></div>`;
  }
  const canInstall = status?.indexer_installed;
  return `<div class="stat"><div class="big-ic warn">${glyph('warn', 22)}</div><div class="grow">
    <div class="s1">The file indexer isn't running</div>
    <div class="s2">Without it, Glint only searches your user folder, and more slowly. ${canInstall ? 'Starting it needs administrator permission once.' : 'It comes with the installer.'}</div>
  </div>${canInstall ? `<button class="btn primary" id="startIndexer">${glyph('bolt', 15)}Start indexer</button>` : ''}</div>`;
}

function contentCard() {
  const c = status?.content;
  if (!c) return '';
  if (!S.content_search) return `<div class="stat"><div class="big-ic warn">${glyph('filetext', 22)}</div><div class="grow"><div class="s1">File-content search is off</div><div class="s2">Turn it on to find documents by the words inside them.</div></div></div>`;
  const busy = c.phase && c.phase !== 'Up to date';
  const m = /\((\d+)\/(\d+)\)/.exec(c.phase || '');
  const pct = m ? (100 * +m[1]) / +m[2] : 0;
  return `<div class="stat"><div class="big-ic ${busy ? 'busy' : 'ok'}">${glyph(busy ? 'refresh' : 'check', 22)}</div><div class="grow">
    <div class="s1">${fmtCount(c.indexed)} documents searchable</div>
    <div class="s2">${esc(c.phase || 'Starting…')}${c.error ? ` · ${esc(c.error)}` : ''}${c.last_full ? ` · last full check ${fmtDate(c.last_full)}` : ''}</div>
    ${busy ? `<div class="progress ${m ? '' : 'indet'}"><div style="${m ? `width:${pct}%` : ''}"></div></div>` : ''}
  </div><button class="btn" id="rebuild">${glyph('refresh', 15)}Rebuild</button></div>`;
}

async function refreshStatus() {
  try {
    status = await call('status');
  } catch {
    return;
  }
  const ic = $('indexerCard');
  if (ic) { ic.innerHTML = indexerCard(); bindStatus(); }
  const cc = $('contentCard');
  if (cc) { cc.innerHTML = contentCard(); bindStatus(); }
  const ver = document.querySelector('.side .ver');
  if (ver) ver.textContent = `Glint ${status.version}`;
}

function bindStatus() {
  const s = $('startIndexer');
  if (s) s.onclick = async () => {
    try { await call('install_indexer'); toast('Starting the indexer…'); setTimeout(refreshStatus, 3000); } catch (e) { if (String(e) !== 'Cancelled') toast(String(e), true); }
  };
  const r = $('rebuild');
  if (r) r.onclick = async () => { await call('rebuild'); toast('Reading your documents again'); setTimeout(refreshStatus, 800); };
}

// ------------------------------------------------------------------ pages
const pages = {
  welcome: () => `
    <div class="hero">
      <div class="logo">${glyph('glint', 44)}</div>
      <h1>Glint is ready</h1>
      <p class="lead" style="max-width:520px">Your new search lives on the Windows key. Apps, files on every drive, the text inside your documents, open windows, settings and a calculator — all in one box.</p>
    </div>
    <div class="feature-grid">
      <div class="feature"><div class="fi">${glyph('keyboard', 18)}</div><b>Press <kbd>⊞ Win</kbd></b><span>Tap the Windows key on its own to open Glint. Win+ shortcuts still work.</span></div>
      <div class="feature"><div class="fi">${glyph('folder', 18)}</div><b><kbd>Win</kbd>+<kbd>E</kbd> → ${S._filepilot_found ? 'File Pilot' : 'Explorer'}</b><span>${S._filepilot_found ? 'File Pilot opens instead of File Explorer.' : 'Install File Pilot and turn this on in Keyboard.'}</span></div>
      <div class="feature"><div class="fi">${glyph('bolt', 18)}</div><b>Right arrow for more</b><span>Show in folder, copy path, run as admin, terminal here, properties…</span></div>
    </div>
    <h2>File indexer</h2>
    <div class="card" id="indexerCard">${indexerCard()}</div>
    <div class="center"><button class="btn primary" id="tryIt">${glyph('search', 15)}Try it now</button><button class="btn" id="toGeneral">Look at settings</button></div>`,

  general: () => `
    <h1>General</h1><p class="lead">How Glint starts and where it appears.</p>
    <div class="card">
      ${item('rocket', 'Start with Windows', 'Glint waits quietly in the background so it opens instantly.', sw('start_with_windows', S.start_with_windows))}
      ${item('layout', 'Show tray icon', 'Off runs Glint fully hidden. You can still reach settings with the gear in the search window, by typing “settings”, or from “Glint Settings” in the Start menu. Type “quit glint” to close it.', sw('show_tray', S.show_tray))}
      ${item('grid', 'Style', 'Pop-up floats in the middle of the screen. Start menu docks Glint above the taskbar, bottom-left, like the classic Windows Start menu, with your pinned apps, All apps, folders and power buttons.', seg('layout', S.layout, [['popup', 'Pop-up'], ['start', 'Start menu']]))}
      ${item('layout', 'Start menu position', 'Where the Start-menu style opens.', seg('start_position', S.start_position, [['left', 'Bottom left'], ['bottom', 'Bottom center'], ['center', 'Center']]))}
      ${item('user', 'Your name', 'Shown at the top of the Start menu. Leave empty to use your Windows user name.', `<input class="inp" id="displayName" value="${esc(S.display_name || '')}" placeholder="${esc(S._user || 'Name')}" maxlength="40"/>`)}
      ${item('monitor', 'Opens on', 'With more than one screen: the one your mouse is on, or always your main screen.', seg('monitor', S.monitor, [['cursor', 'Screen with mouse'], ['primary', 'Main screen']]))}
      ${item('layout', 'Pop-up position', 'Where the pop-up sits on the screen.', seg('position', S.position, [['top', 'Upper third'], ['center', 'Center']]))}
      ${item('clock', 'Show time and date', 'In the search bar, always in view when Glint is open.', sw('show_clock', S.show_clock))}
      ${item('props', 'Preview pane', 'Details, thumbnails and text previews next to the results. <kbd>Ctrl</kbd>+<kbd>P</kbd> toggles it.', sw('show_preview', S.show_preview))}
      ${item('download', 'Check for updates', 'Once a day, quietly. You choose when to install.', sw('check_updates', S.check_updates))}
      ${item('cpu', 'Sleep while hidden', 'Uses less memory while you don’t see it, and wakes instantly.', sw('suspend_hidden', S.suspend_hidden))}
    </div>`,

  keyboard: () => `
    <h1>Keyboard</h1><p class="lead">Shortcuts that open Glint, and what Win+E does.</p>
    <div class="card">
      ${item('glint', 'Windows key opens Glint', 'Tap <kbd>⊞ Win</kbd> by itself. The Start menu is still on the taskbar button, <kbd>Ctrl</kbd>+<kbd>Esc</kbd>, or type “start menu”.', sw('win_key', S.win_key))}
      ${item('keyboard', 'Alt+Space opens Glint', 'A second shortcut, like other launchers.', sw('alt_space', S.alt_space))}
      ${item('folder', 'Win+E opens File Pilot', S.filepilot_path ? `Uses <code>${esc(S.filepilot_path)}</code>` : 'File Pilot wasn\'t found. Pick FPilot.exe to turn this on.', `${sw('win_e_filepilot', S.win_e_filepilot)}`)}
      ${item('external', 'File Pilot location', 'Where FPilot.exe is installed.', `<button class="btn sm" id="pickFp">${glyph('folder', 14)}Choose…</button>`)}
      ${item('folder', 'Open folders with', 'What opens when you pick a folder in Glint.', select('folder_opener', S.folder_opener, [['filepilot', 'File Pilot'], ['explorer', 'File Explorer']]))}
    </div>
    <p class="note">Keys pressed while an app running as administrator is focused don't reach Glint (Windows blocks that), so there the Windows key opens Start as usual.</p>
    <h2>In the search window</h2>
    <div class="card keys-table">
      <div>Open</div><div><kbd>↵</kbd></div>
      <div>Show in folder / file location</div><div><kbd>Ctrl</kbd><kbd>↵</kbd></div>
      <div>Run as administrator</div><div><kbd>Shift</kbd><kbd>↵</kbd></div>
      <div>All actions for the selection</div><div><kbd>→</kbd> or <kbd>Ctrl</kbd><kbd>K</kbd></div>
      <div>Copy path · copy the file itself</div><div><kbd>Ctrl</kbd><kbd>C</kbd> · <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>C</kbd></div>
      <div>Switch filter (All, Apps, Files…)</div><div><kbd>Tab</kbd></div>
      <div>Open the 1st–9th result</div><div><kbd>Ctrl</kbd><kbd>1</kbd>…<kbd>9</kbd></div>
      <div>Previous searches</div><div><kbd>↑</kbd> on an empty box</div>
      <div>Properties · Open with · Terminal here</div><div><kbd>Alt</kbd><kbd>↵</kbd> · <kbd>Ctrl</kbd><kbd>O</kbd> · <kbd>Ctrl</kbd><kbd>T</kbd></div>
      <div>Move to Recycle Bin</div><div><kbd>Shift</kbd><kbd>Del</kbd> twice</div>
      <div>Toggle preview</div><div><kbd>Ctrl</kbd><kbd>P</kbd></div>
      <div>Quick Look (big preview, ←→ to step)</div><div><kbd>Shift</kbd><kbd>Space</kbd></div>
      <div>Pin / unpin the selection</div><div><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>P</kbd></div>
      <div>Clipboard clip: copy without pasting</div><div><kbd>Ctrl</kbd><kbd>↵</kbd></div>
    </div>`,

  search: () => `
    <h1>Search</h1><p class="lead">What shows up, and how much of it.</p>
    <div class="card" id="indexerCard">${indexerCard()}</div>
    <h2>Results</h2>
    <div class="card">
      ${item('file', 'Files per search', 'How many file and folder matches to show in the All view.', `<input type="range" min="3" max="20" value="${S.max_files}" id="maxFiles"/><span class="val" id="maxFilesVal">${S.max_files}</span>`)}
      ${item('globe', 'Offer a web search', 'Adds a “Search the web” row at the bottom. Never shows web results by itself.', sw('web_fallback', S.web_fallback))}
      ${item('search', 'Search engine', 'Used by the web row and by <code>?words</code>.', select('web_engine', S.web_engine, [['google', 'Google'], ['duckduckgo', 'DuckDuckGo'], ['bing', 'Bing'], ['brave', 'Brave'], ['kagi', 'Kagi'], ['perplexity', 'Perplexity']]))}
    </div>
    <h2>Tricks</h2>
    <div class="card keys-table">
      <div>Only PDFs · several types</div><div><code>*.pdf</code> · <code>ext:docx,xlsx</code></div>
      <div>Only folders · only files</div><div><code>folder:</code> · <code>file:</code></div>
      <div>Words in the folder path count too</div><div><code>projects glint main</code></div>
      <div>Exact phrase</div><div><code>"annual report"</code></div>
      <div>Browse a folder (Tab completes)</div><div><code>C:\\</code> · <code>~\\Downloads</code> · <code>%appdata%</code></div>
      <div>Calculator and units</div><div><code>2^10/4</code> · <code>10 km to mi</code> · <code>255 to hex</code></div>
      <div>Run in PowerShell</div><div><code>&gt;ipconfig /all</code></div>
      <div>End a process</div><div><code>kill chrome</code></div>
      <div>Web search</div><div><code>?how to …</code></div>
      <div>Changed recently</div><div><code>dm:today</code> · <code>dm:3d</code> · <code>dm:2025-06</code> · <code>dm:&gt;2025-01-01</code></div>
      <div>By size</div><div><code>size:&gt;100mb</code> · <code>size:1gb..4gb</code> · <code>size:huge</code> · <code>size:empty</code></div>
      <div>Newest big videos</div><div><code>*.mp4 size:&gt;500mb dm:month</code></div>
      <div>Volume, mute, radios</div><div><code>vol 40</code> · <code>mute</code> · <code>wifi off</code> · <code>bt on</code></div>
      <div>System info (Enter copies)</div><div><code>ip</code> · <code>battery</code> · <code>uptime</code> · <code>ram</code> · <code>disk</code></div>
      <div>Move the window you came from</div><div><code>left</code> · <code>right</code> · <code>max</code> · <code>center</code> · <code>third</code> · <code>next monitor</code></div>
      <div>Media</div><div><code>play</code> · <code>pause</code> · <code>next</code> · <code>prev</code></div>
      <div>Clipboard history (ClipVault)</div><div><code>clip invoice</code> · <code>cb </code></div>
      <div>Timers and reminders</div><div><code>timer 10m tea</code> · <code>remind 3pm call</code> · <code>remind in 20m stretch</code> · <code>timers</code></div>
      <div>Per-app volume, output device</div><div><code>vol spotify 30</code> · <code>mute discord</code> · <code>audio headphones</code></div>
      <div>What takes the space (Tab looks inside)</div><div><code>biggest</code> · <code>biggest ~\\Downloads</code></div>
      <div>Latest download · screenshot</div><div><code>dl</code> · <code>ss</code> (Enter copies it) · <code>downloads</code></div>
      <div>Browser</div><div><code>tab youtube</code> · <code>bm recipes</code></div>
    </div>`,

  content: () => `
    <h1>File contents</h1><p class="lead">Find documents by the words inside them: text and code files, Word, Excel, PowerPoint, OpenDocument, EPUB and PDF.</p>
    <div class="card" id="contentCard">${contentCard()}</div>
    <h2>Options</h2>
    <div class="card">
      ${item('filetext', 'Search inside files', 'Reads documents in the background at low priority.', sw('content_search', S.content_search))}
      ${item('hdd', 'Largest file to read', 'Bigger files are still found by name.', `<input type="range" min="5" max="200" step="5" value="${S.content_max_mb}" id="maxMb"/><span class="val" id="maxMbVal">${S.content_max_mb} MB</span>`)}
    </div>
    <h2>Folders to read</h2>
    <div class="card list" id="roots"></div>
    <h2>Leave out</h2>
    <div class="card list" id="excludes"></div>
    <p class="note">Always skipped: AppData, hidden folders, build and package folders (node_modules, target, .git…), online-only OneDrive files (reading them would download them), and files that usually hold secrets (.env, keys, password databases).</p>`,

  extras: () => `
    <h1>Extras & pins</h1><p class="lead">Media controls, clipboard history, and the things you've pinned or given a keyword.</p>
    <div class="card">
      ${item('play', 'Media controls', 'A “Now playing” card when Glint opens, plus play, pause, next and previous. Works with Spotify, browsers and anything that shows in the Windows media flyout.', sw('media_controls', S.media_controls))}
      ${item('clipboard', 'ClipVault history', status?.clipvault ? 'Search what you copied with <code>clip …</code>, the Clipboard tab, or the top matches in All.' : 'ClipVault wasn\'t found on this PC. Install it to search your clipboard history here.', sw('clipvault', S.clipvault))}
      ${item('globe', 'Browser tabs and bookmarks', 'Find open tabs (Enter switches to it) and bookmarks from Chrome, Edge and Brave. <code>tab …</code> and <code>bm …</code> search only those.', sw('browser_search', S.browser_search))}
      ${item('history', 'Browser history', 'Also pages you visited often. Read on this PC only.', sw('browser_history', S.browser_history))}
      ${item('enter', 'Enter pastes the clip', 'Puts it on the clipboard and pastes it into the window you were in. Off just copies it. <kbd>Ctrl</kbd>+<kbd>↵</kbd> always only copies.', sw('clip_paste', S.clip_paste))}
    </div>
    <h2>Pinned</h2>
    <p class="note" style="margin-top:-4px">Pinned items show first when Glint opens and rise to the top of results. Pin anything with <kbd>→</kbd> › Pin, or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>.</p>
    <div class="card list" id="pinList"></div>
    <h2>Global shortcuts</h2>
    <p class="note" style="margin-top:-4px">Open anything with a key combination, from anywhere. Add one with <kbd>→</kbd> › Set a global shortcut….</p>
    <div class="card list" id="hkList"></div>
    <h2>Your scripts</h2>
    <div class="card">${item('terminal', 'Scripts folder', 'Put .ps1, .bat, .cmd, .py or .js files here and they show up in Glint by name. Words after the name are passed as arguments. Enter runs it hidden and shows the result; Shift+Enter runs it in a terminal.', `<button class="btn" id="openScripts">${glyph('folder', 15)}Open folder</button>`)}</div>
    <h2>Keywords</h2>
    <p class="note" style="margin-top:-4px">Type a keyword and its item comes first. Add one with <kbd>→</kbd> › Add keyword….</p>
    <div class="card list" id="kwList"></div>`,

  appearance: () => `
    <h1>Appearance</h1><p class="lead">Make it yours.</p>
    <div class="card">
      ${item('contrast', 'Theme', '', seg('theme', S.theme, [['system', 'System'], ['dark', 'Dark'], ['light', 'Light']]))}
      ${item('palette', 'Accent color', '', `<div class="swatches">${ACCENTS.map((c) => `<button class="swatch ${c.toLowerCase() === (S.accent || '').toLowerCase() ? 'on' : ''}" data-c="${c}" style="background:${c}"></button>`).join('')}<input type="color" class="swatch-custom" id="customAccent" value="${S.accent}" title="Custom"/></div>`)}
      ${item('sparkle', 'Translucent window', 'Blurs what\'s behind the search window. Turn off for a solid background.', sw('blur', S.blur))}
    </div>`,

  privacy: () => `
    <h1>Privacy & data</h1><p class="lead">Everything stays on this PC. Glint never sends your searches or files anywhere. It only goes online for the update check (GitHub), and for “Public IP” when you press Enter on it (api.ipify.org).</p>
    <div class="card">
      ${item('history', 'Search history', 'Glint learns what you open to rank it higher next time, and remembers recent searches for the Up arrow.', `<button class="btn danger" id="clearHist">${glyph('eraser', 15)}Clear</button>`)}
      ${item('folder', 'Data folder', `<code>${esc(status?.data_dir || '')}</code>`, `<button class="btn" id="openData">${glyph('external', 15)}Open</button>`)}
    </div>`,

  about: () => `
    <h1>About Glint</h1><p class="lead">A fast launcher and search for Windows, written in Rust.</p>
    <div class="card">
      <div class="stat"><div class="big-ic busy">${glyph('glint', 22)}</div><div class="grow"><div class="s1">Glint ${esc(S._version)}</div><div class="s2" id="updText">Checking for updates…</div></div><div id="updBtn"></div></div>
    </div>
    <div class="card" style="margin-top:10px">
      ${item('code', 'Source code', 'github.com/JustSad42/glint', `<button class="btn" data-url="https://github.com/JustSad42/glint">${glyph('external', 15)}Open</button>`)}
      ${item('warn', 'Report a problem', '', `<button class="btn" data-url="https://github.com/JustSad42/glint/issues">${glyph('external', 15)}Open</button>`)}
    </div>`,
};

function renderSide() {
  const nav = PAGES.filter((p) => !p.hidden || p.id === page)
    .map((p) => `<button class="nav ${p.id === page ? 'on' : ''}" data-p="${p.id}">${glyph(p.icon, 17)}${p.label}</button>`)
    .join('');
  $('side').innerHTML = nav + `<div class="grow"></div><div class="ver">Glint ${esc(S._version || '')}</div>`;
  $('side').querySelectorAll('.nav').forEach((b) => (b.onclick = () => go(b.dataset.p)));
}

function listEditor(elId, key, opts) {
  const el = $(elId);
  const values = S[key] || [];
  el.innerHTML =
    (values.length ? values.map((v, i) => `<div class="li">${glyph(opts.icon, 16)}<span class="p" title="${esc(v)}">${esc(v)}</span><button class="x" data-i="${i}" title="Remove">${glyph('x', 14)}</button></div>`).join('')
      : `<div class="li"><span class="p" style="color:var(--text-3)">${opts.empty}</span></div>`) +
    `<div class="li-add">${opts.pick ? `<button class="btn sm" id="${elId}Add">${glyph('folder', 14)}Add folder…</button>` : `<input class="inp wide" id="${elId}Inp" placeholder="${opts.placeholder}"/><button class="btn sm" id="${elId}Add">Add</button>`}</div>`;
  el.querySelectorAll('.x').forEach((b) => (b.onclick = () => {
    const next = values.filter((_, i) => i !== +b.dataset.i);
    save({ [key]: next }, 'Saved').then(() => listEditor(elId, key, opts));
  }));
  $(`${elId}Add`).onclick = async () => {
    let v;
    if (opts.pick) v = await call('pick_folder');
    else v = $(`${elId}Inp`).value.trim();
    if (!v || values.includes(v)) return;
    await save({ [key]: [...values, v] }, 'Saved');
    listEditor(elId, key, opts);
  };
  const inp = $(`${elId}Inp`);
  if (inp) inp.onkeydown = (e) => { if (e.key === 'Enter') $(`${elId}Add`).click(); };
}

async function loadUpdate(force) {
  const t = $('updText');
  if (!t) return;
  try {
    const u = await call('update_check', { force });
    const b = $('updBtn');
    if (u.error) { t.textContent = u.error; b.innerHTML = `<button class="btn" id="recheck">Check again</button>`; }
    else if (u.available) {
      t.textContent = `Version ${u.latest} is available.`;
      b.innerHTML = u.can_install ? `<button class="btn primary" id="install">${glyph('download', 15)}Install ${esc(u.latest)}</button>` : `<button class="btn" data-url="${esc(u.page)}">Download</button>`;
    } else { t.textContent = `You have the newest version.`; b.innerHTML = `<button class="btn" id="recheck">Check again</button>`; }
    const rc = $('recheck');
    if (rc) rc.onclick = () => { t.textContent = 'Checking…'; loadUpdate(true); };
    const ins = $('install');
    if (ins) ins.onclick = async () => {
      ins.disabled = true; ins.textContent = 'Downloading…';
      try { await call('update_install'); t.textContent = 'Installing — Glint will restart.'; } catch (e) { toast(String(e), true); ins.disabled = false; ins.textContent = 'Install'; }
    };
    bindUrls();
  } catch (e) { t.textContent = String(e); }
}

function bindUrls() {
  document.querySelectorAll('[data-url]').forEach((b) => (b.onclick = () => call('open_url', { url: b.dataset.url })));
}

function go(p) {
  page = p;
  history.replaceState(null, '', `#${p}`);
  renderSide();
  $('content').innerHTML = `<div class="page">${(pages[p] || pages.general)()}</div>`;
  $('content').scrollTop = 0;
  bind();
  bindStatus();
  bindUrls();
  if (p === 'welcome') {
    $('tryIt').onclick = () => call('show');
    $('toGeneral').onclick = () => go('general');
  }
  if (p === 'keyboard') {
    $('pickFp').onclick = async () => {
      const f = await call('pick_file');
      if (f) { await save({ filepilot_path: f, win_e_filepilot: true }, 'File Pilot set'); go('keyboard'); }
    };
  }
  if (p === 'general') {
    const dn = $('displayName');
    dn.onchange = () => save({ display_name: dn.value.trim() }, 'Saved');
    dn.onkeydown = (e) => { if (e.key === 'Enter') dn.blur(); };
  }
  if (p === 'search') {
    const r = $('maxFiles');
    r.oninput = () => ($('maxFilesVal').textContent = r.value);
    r.onchange = () => save({ max_files: +r.value }, 'Saved');
  }
  if (p === 'content') {
    const r = $('maxMb');
    r.oninput = () => ($('maxMbVal').textContent = `${r.value} MB`);
    r.onchange = () => save({ content_max_mb: +r.value }, 'Saved');
    listEditor('roots', 'content_roots', { icon: 'folder', pick: true, empty: 'No folders — nothing will be read.' });
    listEditor('excludes', 'content_excludes', { icon: 'x', placeholder: 'A folder name or part of a path, e.g. Archive or \\Old Projects\\', empty: 'Nothing extra is left out.' });
  }
  if (p === 'appearance') {
    document.querySelectorAll('.swatch').forEach((b) => (b.onclick = () => {
      document.querySelectorAll('.swatch').forEach((x) => x.classList.toggle('on', x === b));
      save({ accent: b.dataset.c });
    }));
    const ca = $('customAccent');
    ca.oninput = () => document.documentElement.style.setProperty('--accent', ca.value);
    ca.onchange = () => { document.querySelectorAll('.swatch').forEach((x) => x.classList.remove('on')); save({ accent: ca.value }); };
  }
  if (p === 'privacy') {
    $('clearHist').onclick = async () => { await call('clear_history'); toast('History cleared'); };
    $('openData').onclick = () => call('open_path', { path: status?.data_dir });
  }
  if (p === 'about') loadUpdate(false);
  if (p === 'extras') loadPins();
}

const KIND_ICON = { app: 'app', file: 'file', folder: 'folder', action: 'bolt', url: 'globe', layout: 'layout', toggle: 'sliders', command: 'terminal' };
async function loadPins() {
  let d;
  try { d = await call('pins_list'); } catch { return; }
  const pl = $('pinList');
  const kl = $('kwList');
  if (!pl || !kl) return;
  pl.innerHTML = d.pinned.length
    ? d.pinned.map((x) => `<div class="li">${glyph(KIND_ICON[x.kind] || 'pin', 16)}<span class="p">${esc(x.title)}</span><button class="x" data-key="${esc(x.key)}" title="Unpin">${glyph('x', 14)}</button></div>`).join('')
    : `<div class="li"><span class="p" style="color:var(--text-3)">Nothing pinned yet.</span></div>`;
  kl.innerHTML = d.keywords.length
    ? d.keywords.map((x) => `<div class="li"><code class="kw">${esc(x.keyword)}</code><span class="p">${esc(x.title)}</span><button class="x" data-word="${esc(x.keyword)}" title="Remove">${glyph('x', 14)}</button></div>`).join('')
    : `<div class="li"><span class="p" style="color:var(--text-3)">No keywords yet.</span></div>`;
  pl.querySelectorAll('.x').forEach((b) => (b.onclick = async () => { await call('unpin', { key: b.dataset.key }); toast('Unpinned'); loadPins(); }));
  const hl = $('hkList');
  if (hl) {
    hl.innerHTML = (d.hotkeys || []).length
      ? d.hotkeys.map((x) => `<div class="li"><code class="kw">${esc(x.combo)}</code><span class="p">${esc(x.title)}</span><button class="x" data-combo="${esc(x.combo)}" title="Remove">${glyph('x', 14)}</button></div>`).join('')
      : `<div class="li"><span class="p" style="color:var(--text-3)">No shortcuts yet.</span></div>`;
    hl.querySelectorAll('.x').forEach((b) => (b.onclick = async () => { await call('hotkey_remove', { combo: b.dataset.combo }); toast('Shortcut removed'); loadPins(); }));
  }
  const os = $('openScripts');
  if (os) os.onclick = () => call('open_scripts');
  kl.querySelectorAll('.x').forEach((b) => (b.onclick = async () => { await call('keyword_remove', { word: b.dataset.word }); toast('Keyword removed'); loadPins(); }));
}

listen('goto', (p) => go(p));
listen('settings-changed', (s) => { S = { ...S, ...s }; applyTheme(S); });
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(S));
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') T.window.getCurrentWindow().close(); });

(async () => {
  S = await call('settings_get');
  applyTheme(S);
  try { status = await call('status'); } catch {}
  go(page);
  setInterval(refreshStatus, 1500);
})();
