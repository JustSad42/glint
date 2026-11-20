// Glint launcher: search box, ranked results, preview pane and action menu.
const $ = (id) => document.getElementById(id);
const qEl = $('q');
const resultsEl = $('results');
const previewEl = $('preview');
const menuEl = $('menu');
const shellEl = $('shell');

const FILTERS = [
  { id: 'all', label: 'All', icon: 'sparkle' },
  { id: 'apps', label: 'Apps', icon: 'grid' },
  { id: 'files', label: 'Files', icon: 'file' },
  { id: 'folders', label: 'Folders', icon: 'folder' },
  { id: 'contents', label: 'In files', icon: 'filetext' },
  { id: 'windows', label: 'Windows', icon: 'window' },
  { id: 'actions', label: 'Commands', icon: 'bolt' },
  { id: 'clipboard', label: 'Clipboard', icon: 'clipboard' },
];
const tabs = () => FILTERS.filter((f) => f.id !== 'clipboard' || (S.settings.clipvault !== false && S.status?.clipvault));
const fileSrc = (p) => window.__TAURI__.core.convertFileSrc(p);

const S = {
  settings: {},
  filter: 'all',
  sections: [],
  flat: [],
  sel: 0,
  inflight: false,
  pending: false,
  lastQuery: null,
  mode: null,
  confirmKey: null,
  menu: null, // { items, sel, filter }
  hiddenAt: 0,
  queryHistory: [],
  historyIdx: -1,
  status: null,
  detailsFor: null,
  mouseMoved: false,
  vscode: false,
};

// ------------------------------------------------------------------ setup
$('barIcon').innerHTML = glyph('search', 20);
$('settingsBtn').innerHTML = glyph('gear', 18);
$('settingsBtn').onclick = () => call('open_settings', {});

function renderTabs() {
  $('tabs').innerHTML =
    tabs().map((f) => `<button class="tab ${S.filter === f.id ? 'on' : ''}" data-f="${f.id}">${glyph(f.icon, 14)}${f.label}</button>`).join('') +
    `<span class="spacer"></span><span class="tab-hint"><kbd>Tab</kbd> switch</span>`;
  $('tabs').querySelectorAll('.tab').forEach((b) => (b.onclick = () => setFilter(b.dataset.f)));
}

function setFilter(f) {
  if (S.filter === f) return;
  S.filter = f;
  renderTabs();
  search(true);
  qEl.focus();
}

function cycleFilter(dir) {
  const list = tabs();
  const i = list.findIndex((f) => f.id === S.filter);
  setFilter(list[(i + dir + list.length) % list.length].id);
}

async function loadSettings() {
  S.settings = await call('settings_get');
  applyTheme(S.settings);
  shellEl.classList.toggle('no-preview', !S.settings.show_preview);
}

// ------------------------------------------------------------------ search
async function search(force = false) {
  const q = qEl.value;
  if (!force && q === S.lastQuery) return;
  if (S.inflight) {
    S.pending = true;
    return;
  }
  S.inflight = true;
  $('barIcon').classList.add('busy');
  try {
    const r = await call('search', { q, filter: S.filter });
    S.lastQuery = q;
    // a newer query arrived while this one ran: show this one only if nothing better is coming
    if (q === qEl.value || !S.pending) render(r, q);
  } catch (e) {
    console.error(e);
  } finally {
    S.inflight = false;
    $('barIcon').classList.remove('busy');
    if (S.pending) {
      S.pending = false;
      search();
    }
  }
}

function render(r, q) {
  const prevKey = S.flat[S.sel]?.key;
  S.sections = r.sections || [];
  S.mode = r.mode || null;
  S.live = r.live;
  S.ms = r.ms;
  S.flat = [];
  S.confirmKey = null;
  for (const s of S.sections) for (const it of s.items) S.flat.push(it);
  // keep the selection on the same item while typing refines results, else go to the top
  const keep = q === S.renderedQuery ? S.flat.findIndex((x) => x.key === prevKey) : -1;
  S.sel = keep >= 0 ? keep : 0;
  S.renderedQuery = q;
  renderMode();
  renderResults(q);
  renderFooter();
  updatePreview();
}

function renderMode() {
  const m = $('mode');
  const label = { command: 'Command', kill: 'End process', web: 'Web search', path: 'Browse', clip: 'Clipboard' }[S.mode];
  m.hidden = !label;
  m.textContent = label || '';
}

function highlight(text, q) {
  const t = String(text ?? '');
  const words = (q || '').toLowerCase().split(/\s+/).filter((w) => w && !w.includes(':') && !w.startsWith('*') && !w.startsWith('>'));
  if (!words.length) return esc(t);
  const lower = t.toLowerCase();
  const marks = new Array(t.length).fill(false);
  let any = false;
  for (const w of words) {
    let i = lower.indexOf(w);
    if (i >= 0) {
      for (let k = i; k < i + w.length; k++) marks[k] = true;
      any = true;
    }
  }
  if (!any && words.length === 1) {
    // initials / letters in order
    const w = words[0];
    let j = 0;
    for (let i = 0; i < t.length && j < w.length; i++) {
      if (lower[i] === w[j]) {
        marks[i] = true;
        j++;
      }
    }
    if (j < w.length) marks.fill(false);
  }
  let out = '';
  let open = false;
  for (let i = 0; i < t.length; i++) {
    if (marks[i] && !open) { out += '<mark>'; open = true; }
    if (!marks[i] && open) { out += '</mark>'; open = false; }
    out += esc(t[i]);
  }
  if (open) out += '</mark>';
  return out;
}

const KIND_GLYPH = { app: 'app', file: 'file', folder: 'folder', content: 'filetext', window: 'window', process: 'activity', calc: 'calc', web: 'search', url: 'globe', command: 'terminal', action: 'bolt', color: 'palette', hint: 'info', toggle: 'sliders', info: 'info', layout: 'layout', media: 'play', clip: 'clipboard' };
const KIND_BADGE = { app: 'App', window: 'Window', content: 'Match', action: 'Command', process: 'Process', folder: 'Folder', layout: 'Layout', clip: 'Clip' };
const PINNABLE = ['app', 'file', 'folder', 'action', 'url', 'layout', 'toggle', 'command'];

function iconHtml(it) {
  if (it.kind === 'color') return `<div class="ico swatch" style="background:${esc(it.swatch)}"></div>`;
  if (it.icon) return `<div class="ico"><img src="${esc(it.icon)}" loading="lazy" decoding="async" data-fallback="${esc(it.glyph || KIND_GLYPH[it.kind] || 'file')}" /></div>`;
  const accent = ['calc', 'web', 'url', 'command', 'toggle', 'info', 'layout', 'media'].includes(it.kind) ? 'accent' : '';
  return `<div class="ico glyphy ${accent}">${glyph(it.glyph || KIND_GLYPH[it.kind] || 'sparkle', 18)}</div>`;
}

function goLabel(it) {
  if (it.confirm && S.confirmKey === it.key) return 'Press again';
  if (it.kind === 'media') return { toggle: it.playing ? 'Pause' : 'Play', next: 'Next', prev: 'Previous' }[it.op] || 'Play';
  if (it.kind === 'info') return it.op === 'public_ip' ? 'Look up' : it.copy ? 'Copy' : '';
  if (it.kind === 'clip') return S.settings.clip_paste === false ? 'Copy' : 'Paste';
  return { window: 'Switch', calc: 'Copy', color: 'Copy', process: 'End', web: 'Search', url: 'Open', command: 'Run', action: 'Run', hint: '', toggle: 'Apply', layout: 'Apply' }[it.kind] ?? 'Open';
}

function mediaCardHtml(it, i) {
  return `<div class="row media-card ${i === S.sel ? 'sel' : ''}" data-i="${i}">
    <div class="cover"><img src="${esc(it.icon)}" onerror="this.parentElement.classList.add('none');this.remove()" />${glyph('play', 20)}</div>
    <div class="txt"><div class="title">${esc(it.title)}</div><div class="sub">${esc(it.subtitle)}</div>
      <div class="eq ${it.playing ? 'on' : ''}"><i></i><i></i><i></i><i></i></div></div>
    <div class="mc-btns">
      <button data-op="prev" title="Previous">${glyph('prev', 16)}</button>
      <button data-op="toggle" class="main" title="${it.playing ? 'Pause' : 'Play'}">${glyph(it.playing ? 'pause' : 'play', 18)}</button>
      <button data-op="next" title="Next">${glyph('next', 16)}</button>
    </div>
  </div>`;
}

function fileMeta(it) {
  if (it.kind !== 'file' && it.kind !== 'folder') return '';
  const parts = [];
  if (it.kind === 'file' && it.size != null) parts.push(fmtSize(it.size));
  if (it.mtime) parts.push(fmtShortDate(it.mtime));
  return parts.length ? `<span class="fmeta">${esc(parts.join(' · '))}</span>` : '';
}

function rowHtml(it, i, q) {
  if (it.key === 'media:now') return mediaCardHtml(it, i);
  const danger = it.confirm && S.confirmKey === it.key;
  const badge = it.recent ? 'Recent' : KIND_BADGE[it.kind];
  const titleHtml = it.kind === 'calc' ? esc(it.title) : highlight(it.title, q);
  const sub = danger ? `Press Enter again to confirm` : it.subtitle;
  return `<div class="row ${it.kind} ${i === S.sel ? 'sel' : ''} ${danger ? 'danger' : ''}" data-i="${i}">
    ${iconHtml(it)}
    <div class="txt"><div class="title">${it.pinned ? `<span class="pinmark">${glyph('pin', 12)}</span>` : ''}${titleHtml}</div>${sub ? `<div class="sub">${esc(sub)}</div>` : ''}</div>
    <div class="meta">
      ${fileMeta(it)}
      ${badge && S.filter === 'all' ? `<span class="badge">${badge}</span>` : ''}
      ${i < 9 ? `<span class="num">Ctrl ${i + 1}</span>` : ''}
      <span class="go">${goLabel(it) ? `${esc(goLabel(it))} <kbd>↵</kbd>` : ''}</span>
    </div>
  </div>`;
}

const TIPS = [
  { q: '', icon: 'grid', t1: 'Apps & settings', t2: 'Type a name — "vsc" finds VS Code' },
  { q: '*.pdf ', icon: 'file', t1: 'Files anywhere', t2: '<code>*.pdf</code> <code>folder:</code> <code>ext:docx</code>' },
  { q: '=', icon: 'calc', t1: 'Calculator', t2: '<code>2^10/4</code> <code>10 km to mi</code>' },
  { q: '>', icon: 'terminal', t1: 'Run a command', t2: '<code>&gt;ipconfig</code> in PowerShell' },
  { q: 'kill ', icon: 'activity', t1: 'End a process', t2: '<code>kill chrome</code>' },
  { q: '~\\', icon: 'folder', t1: 'Browse folders', t2: '<code>C:\\</code> <code>~\\Downloads</code>, Tab completes' },
  { q: '?', icon: 'globe', t1: 'Web search', t2: '<code>?rust traits</code>' },
  { q: '#7c6cff', icon: 'palette', t1: 'Colors', t2: '<code>#7c6cff</code> <code>rgb(…)</code>' },
  { q: 'dm:week ', icon: 'clock', t1: 'Recent & big files', t2: '<code>dm:today</code> <code>size:>100mb</code>' },
  { q: 'vol ', icon: 'sliders', t1: 'Quick toggles', t2: '<code>vol 40</code> <code>wifi off</code> <code>ip</code>' },
  { q: 'left', icon: 'layout', t1: 'Snap a window', t2: '<code>left</code> <code>center</code> <code>next monitor</code>' },
  { q: 'clip ', icon: 'clipboard', t1: 'Clipboard history', t2: '<code>clip invoice</code> from ClipVault' },
];

function renderResults(q) {
  if (!S.flat.length) {
    if (!q.trim()) {
      resultsEl.innerHTML =
        `<div class="sec">Get started</div><div class="tips">` +
        TIPS.map((t, i) => `<div class="tip" data-tip="${i}"><div class="ico glyphy accent">${glyph(t.icon, 18)}</div><div><div class="t1">${t.t1}</div><div class="t2">${t.t2}</div></div></div>`).join('') +
        `</div>`;
      resultsEl.querySelectorAll('.tip').forEach((el) => (el.onclick = () => {
        const t = TIPS[+el.dataset.tip];
        if (!t.q) return qEl.focus();
        qEl.value = t.q;
        qEl.focus();
        search(true);
      }));
    } else {
      const off = S.live === false;
      resultsEl.innerHTML = `<div class="empty"><div class="big">${glyph('search', 40)}</div><h3>No results for “${esc(q.trim())}”</h3>
        <p>${off ? 'The file indexer isn’t running, so only your user folder was searched. ' : ''}Try fewer words, or press <kbd>Tab</kbd> to search only files, folders or file contents.</p></div>`;
    }
    return;
  }
  let html = '';
  let i = 0;
  for (const s of S.sections) {
    if (!s.items.length) continue;
    html += `<div class="sec">${esc(s.title)}</div>`;
    for (const it of s.items) html += rowHtml(it, i++, q);
  }
  resultsEl.innerHTML = html;
  resultsEl.querySelectorAll('.mc-btns button').forEach((b) => (b.onclick = (e) => {
    e.stopPropagation();
    const row = b.closest('.row');
    const it = S.flat[+row.dataset.i];
    runAction({ ...it, op: b.dataset.op }, { mode: 'default' });
  }));
  resultsEl.querySelectorAll('img[data-fallback]').forEach((img) => {
    img.onerror = () => {
      const box = img.parentElement;
      box.classList.add('glyphy');
      box.innerHTML = glyph(img.dataset.fallback, 18);
    };
  });
  scrollToSel();
}

function updateSel(n) {
  if (!S.flat.length) return;
  S.sel = (n + S.flat.length) % S.flat.length;
  if (S.confirmKey && S.flat[S.sel]?.key !== S.confirmKey) {
    S.confirmKey = null;
    renderResults(qEl.value);
  }
  resultsEl.querySelectorAll('.row.sel').forEach((r) => r.classList.remove('sel'));
  const row = resultsEl.querySelector(`.row[data-i="${S.sel}"]`);
  row?.classList.add('sel');
  scrollToSel();
  updatePreview();
  renderFooter();
}

function scrollToSel() {
  const row = resultsEl.querySelector(`.row[data-i="${S.sel}"]`);
  if (!row) return;
  if (S.sel === 0) resultsEl.scrollTop = 0;
  else row.scrollIntoView({ block: 'nearest' });
}

// ------------------------------------------------------------------ preview
let previewTimer = 0;
function updatePreview() {
  if (!S.settings.show_preview) return;
  clearTimeout(previewTimer);
  const it = S.flat[S.sel];
  if (!it) {
    previewEl.innerHTML = '';
    S.detailsFor = null;
    return;
  }
  renderPreview(it, null);
  previewTimer = setTimeout(async () => {
    if (!['file', 'folder', 'content', 'app', 'window', 'process', 'action', 'url', 'layout', 'toggle', 'command'].includes(it.kind)) return;
    const key = it.key;
    S.detailsFor = key;
    S.details = null;
    try {
      const d = await call('details', { item: it });
      if (S.detailsFor === key && S.flat[S.sel]?.key === key) {
        S.details = d;
        renderPreview(it, d);
      }
    } catch {}
  }, 70);
}

function prettyPath(p) {
  const home = S.status?.home;
  if (home && p.toLowerCase().startsWith(home.toLowerCase())) return '~' + p.slice(home.length);
  return p;
}

function fact(k, v) {
  return v ? `<div class="pv-fact"><span>${k}</span><span>${esc(v)}</span></div>` : '';
}

function markText(text, marks) {
  if (!marks?.length) return esc(text);
  let out = '';
  let last = 0;
  // offsets are UTF-8 bytes from Rust; convert by encoding
  const enc = new TextEncoder().encode(text);
  const dec = new TextDecoder();
  for (const [a, b] of marks) {
    if (a < last) continue;
    out += esc(dec.decode(enc.slice(last, a))) + '<mark>' + esc(dec.decode(enc.slice(a, b))) + '</mark>';
    last = b;
  }
  return out + esc(dec.decode(enc.slice(last)));
}

function mediaHero(d) {
  const src = fileSrc(d.path);
  if (d.media === 'image') return `<img class="pv-media" src="${esc(src)}" data-dims="1" onerror="this.remove()" />`;
  if (d.media === 'video') return `<video class="pv-media" src="${esc(src)}" controls muted preload="metadata" data-dims="1"></video>`;
  if (d.media === 'audio') return `<div class="pv-audio"><div class="pv-ico glyphy">${glyph('volume', 30)}</div><audio src="${esc(src)}" controls preload="metadata" data-dims="1"></audio></div>`;
  return '';
}

function wireMedia() {
  previewEl.querySelectorAll('[data-dims]').forEach((m) => {
    const put = (txt) => {
      const el = previewEl.querySelector('[data-fact="dims"]');
      if (el && txt) { el.hidden = false; el.lastElementChild.textContent = txt; }
    };
    if (m.tagName === 'IMG') m.onload = () => put(`${m.naturalWidth} × ${m.naturalHeight}`);
    else m.onloadedmetadata = () => put([m.videoWidth ? `${m.videoWidth} × ${m.videoHeight}` : '', fmtDuration(m.duration)].filter(Boolean).join(' · '));
  });
}

function renderPreview(it, d) {
  const acts = actionsFor(it, d).slice(0, 5);
  let hero;
  if (it.kind === 'clip') {
    const img = it.image ? `<img class="pv-media" src="${esc(fileSrc(it.image))}" data-dims="1" onerror="this.remove()" />` : '';
    hero = `${img}<div class="pv-kind">${esc(it.subtitle)}</div>`;
    if (it.clipkind !== 'Image' && it.text) hero += `<div class="pv-text clip">${esc(it.text)}</div>`;
    hero += img ? `<div class="pv-facts"><div class="pv-fact" data-fact="dims" hidden><span>Size</span><span></span></div></div>` : '';
  } else if (it.key === 'media:now') {
    hero = `<div class="pv-hero"><div class="pv-cover"><img src="${esc(it.icon)}" onerror="this.parentElement.classList.add('none');this.remove()" />${glyph('play', 36)}</div><div class="pv-name">${esc(it.title)}</div><div class="pv-kind">${esc(it.subtitle)}</div></div>`;
  } else if (it.kind === 'calc') {
    hero = `<div class="pv-calc">${esc(it.title)}</div><div class="pv-kind" style="text-align:center">${esc(it.subtitle)}</div>`;
  } else if (it.kind === 'color') {
    hero = `<div class="pv-swatch" style="background:${esc(it.swatch)}"></div><div class="pv-name">${esc(it.title)}</div><div class="pv-facts">${(it.alts || []).map((a, i) => fact(['Hex', 'RGB', 'HSL'][i], a)).join('')}</div>`;
  } else {
    const big = d?.media ? mediaHero(d) : d?.thumb ? `<img class="pv-thumb" src="${esc(d.thumb)}" onerror="this.remove()" />` : '';
    const ico = it.icon ? `<div class="pv-ico"><img src="${esc(it.icon)}" onerror="this.parentElement.classList.add('glyphy');this.outerHTML=glyph('${KIND_GLYPH[it.kind] || 'file'}',32)" /></div>`
      : `<div class="pv-ico glyphy">${glyph(it.glyph || KIND_GLYPH[it.kind] || 'sparkle', 30)}</div>`;
    const kindName = { app: 'Application', file: 'File', folder: 'Folder', content: 'Text match', window: 'Open window', process: 'Running process', action: 'Command', web: 'Web search', url: 'Website', command: 'PowerShell command' }[it.kind] || '';
    hero = `<div class="pv-hero">${big || ico}<div class="pv-name">${esc(it.title)}</div><div class="pv-kind">${esc(kindName)}</div></div>`;
  }
  let body = '';
  if (d) {
    if (d.missing) body += `<div class="pv-kind" style="color:var(--danger)">This file no longer exists.</div>`;
    const facts = [];
    if (d.is_dir === false) facts.push(fact('Size', fmtSize(d.size)));
    if (d.is_dir && d.count != null) facts.push(fact('Items', d.count >= 400 ? '400+' : String(d.count)));
    facts.push(fact('Modified', fmtDate(d.modified)));
    if (d.created) facts.push(fact('Created', fmtDate(d.created)));
    if (d.media) facts.push(`<div class="pv-fact" data-fact="dims" hidden><span>${d.media === 'image' ? 'Dimensions' : d.media === 'audio' ? 'Length' : 'Video'}</span><span></span></div>`);
    if (d.version?.description) facts.push(fact('Description', d.version.description));
    if (d.version?.company) facts.push(fact('Publisher', d.version.company));
    if (d.version?.version) facts.push(fact('Version', d.version.version));
    if (d.target) facts.push(fact('Target', prettyPath(d.target)));
    if (d.opens_with) facts.push(fact('Opens with', d.opens_with));
    if (d.archive) facts.push(fact('Contains', `${fmtCount(d.archive.count)} items · ${fmtSize(d.archive.unpacked)} unpacked`));
    if (d.pretty) facts.push(fact('Where', d.pretty));
    if (d.keywords?.length) facts.push(fact('Keyword', d.keywords.join(', ')));
    if ((it.kind === 'app' || it.kind === 'window' || it.kind === 'process') && d.path) facts.push(fact('Program', prettyPath(d.path)));
    if (facts.join('')) body += `<div class="pv-facts">${facts.join('')}</div>`;
    if (d.snippet?.text) body += `<div class="pv-text pv-snippet">${markText(d.snippet.text, d.snippet.marks)}</div>`;
    else if (d.text) body += `<div class="pv-text">${esc(d.text)}</div>`;
    if (d.archive?.entries?.length) body += `<div class="pv-children">${d.archive.entries.map((c) => `<div class="pv-child">${glyph(c.dir ? 'folder' : 'file', 14)}${esc(c.name)}</div>`).join('')}</div>`;
    if (d.children?.length) body += `<div class="pv-children">${d.children.map((c) => `<div class="pv-child">${glyph(c.dir ? 'folder' : 'file', 14)}${esc(c.name)}</div>`).join('')}</div>`;
  } else if (it.subtitle && !['calc', 'color', 'clip'].includes(it.kind) && it.key !== 'media:now') {
    body += `<div class="pv-facts">${fact(it.kind === 'file' || it.kind === 'folder' || it.kind === 'content' ? 'Where' : 'Details', it.subtitle)}</div>`;
  }
  const actHtml = acts.length > 1
    ? `<div class="pv-actions">${acts.map((a, i) => `<button class="pv-act ${a.danger ? 'danger' : ''}" data-a="${i}">${glyph(a.icon, 15)}${esc(a.label)}${a.keys ? `<span class="keys" style="margin-left:auto;display:flex;gap:3px">${a.keys.map((k) => `<kbd>${k}</kbd>`).join('')}</span>` : ''}</button>`).join('')}</div>`
    : '';
  previewEl.querySelectorAll('video,audio').forEach((m) => m.pause());
  previewEl.innerHTML = hero + body + actHtml;
  wireMedia();
  previewEl.querySelectorAll('.pv-act').forEach((b) => (b.onclick = () => runAction(it, acts[+b.dataset.a])));
}

// ------------------------------------------------------------------ actions
const RUNNABLE = ['exe', 'bat', 'cmd', 'ps1', 'msi', 'lnk', 'com'];

function actionsFor(it, d = it.key === S.detailsFor ? S.details : null) {
  const base = baseActions(it);
  if (!PINNABLE.includes(it.kind)) return base;
  const A = (label, icon, mode, keys, extra = {}) => ({ label, icon, mode, keys, ...extra });
  const pinned = d ? d.pinned : it.pinned;
  const extra = [pinned ? A('Unpin', 'pin', 'unpin', ['Ctrl', 'Shift', 'P']) : A('Pin to top', 'pin', 'pin', ['Ctrl', 'Shift', 'P']), A('Add keyword…', 'tag', 'keyword-prompt', [])];
  for (const w of d?.keywords || []) extra.push(A(`Remove keyword “${w}”`, 'eraser', `unkeyword:${w}`, []));
  // keep the dangerous/forget entries last
  const tail = base.filter((a) => a.danger || a.mode === 'forget');
  return [...base.filter((a) => !tail.includes(a)), ...extra, ...tail];
}

function baseActions(it) {
  const ext = (it.path || '').split('.').pop().toLowerCase();
  const fp = S.settings.folder_opener === 'filepilot';
  const A = (label, icon, mode, keys, extra = {}) => ({ label, icon, mode, keys, ...extra });
  const recent = it.recent ? [A('Remove from Recent', 'eraser', 'forget', ['Ctrl', 'Del'])] : [];
  switch (it.kind) {
    case 'app':
      return [
        A('Open', 'external', 'default', ['↵']),
        ...(it.store ? [] : [A('Run as administrator', 'admin', 'admin', ['Shift', '↵'])]),
        ...(it.path || it.lnk ? [A('Open file location', 'folder', 'reveal', ['Ctrl', '↵']), A('Copy path', 'copy', 'copy-path', ['Ctrl', 'C'])] : []),
        ...(it.lnk ? [A('Properties', 'props', 'properties', ['Alt', '↵'])] : []),
        ...recent,
      ];
    case 'file':
    case 'content':
      return [
        A('Open', 'external', 'default', ['↵']),
        A('Show in folder', 'folder', 'reveal', ['Ctrl', '↵']),
        A('Copy path', 'copy', 'copy-path', ['Ctrl', 'C']),
        A('Copy file', 'copy', 'copy-file', ['Ctrl', 'Shift', 'C']),
        A('Open with…', 'grid', 'open-with', ['Ctrl', 'O']),
        ...(RUNNABLE.includes(ext) ? [A('Run as administrator', 'admin', 'admin', ['Shift', '↵'])] : []),
        ...(S.vscode ? [A('Open in VS Code', 'code', 'vscode', [])] : []),
        A('Open terminal here', 'terminal', 'terminal', ['Ctrl', 'T']),
        A('Copy name', 'copy', 'copy-name', []),
        A('Properties', 'props', 'properties', ['Alt', '↵']),
        A('Move to Recycle Bin', 'trash', 'recycle', ['Shift', 'Del'], { danger: true }),
        ...recent,
      ];
    case 'folder':
      return [
        A('Open', 'external', 'default', ['↵']),
        A('Show in parent folder', 'folder', 'reveal', ['Ctrl', '↵']),
        A('Open terminal here', 'terminal', 'terminal', ['Ctrl', 'T']),
        ...(S.vscode ? [A('Open in VS Code', 'code', 'vscode', [])] : []),
        ...(fp ? [A('Open in File Explorer', 'folder', 'explorer', [])] : []),
        A('Copy path', 'copy', 'copy-path', ['Ctrl', 'C']),
        A('Copy folder', 'copy', 'copy-file', ['Ctrl', 'Shift', 'C']),
        A('Properties', 'props', 'properties', ['Alt', '↵']),
        A('Move to Recycle Bin', 'trash', 'recycle', ['Shift', 'Del'], { danger: true }),
        ...recent,
      ];
    case 'window':
      return [A('Switch to window', 'window', 'default', ['↵']), A('Close window', 'x', 'close', ['Ctrl', 'W']), A('Show program in folder', 'folder', 'reveal', ['Ctrl', '↵'])];
    case 'process':
      return [A('End process', 'x', 'default', ['↵'], { danger: true })];
    case 'calc':
      return [A('Copy result', 'copy', 'default', ['↵'])];
    case 'color':
      return (it.alts || []).map((a, i) => A(`Copy ${a}`, 'copy', i ? `alt:${a}` : 'default', i ? [] : ['↵']));
    case 'command':
      return [A('Run', 'terminal', 'default', ['↵']), A('Run as administrator', 'admin', 'admin', ['Shift', '↵'])];
    case 'url':
    case 'web':
      return [A('Open in browser', 'globe', 'default', ['↵'])];
    case 'action':
      return [A('Run', 'bolt', 'default', ['↵']), ...recent];
    case 'toggle':
      return [A('Apply', it.glyph || 'sliders', 'default', ['↵'])];
    case 'info':
      return it.copy || it.op ? [A(it.op === 'public_ip' ? 'Look up and copy' : 'Copy', 'copy', 'default', ['↵'])] : [];
    case 'layout':
      return [A('Apply to window', 'layout', 'default', ['↵'])];
    case 'media':
      return it.key === 'media:now'
        ? [A(it.playing ? 'Pause' : 'Play', it.playing ? 'pause' : 'play', 'default', ['↵']), A('Next track', 'next', 'media-next', []), A('Previous track', 'prev', 'media-prev', [])]
        : [A(goLabel(it), it.glyph || 'play', 'default', ['↵'])];
    case 'clip':
      return S.settings.clip_paste === false
        ? [A('Copy to clipboard', 'copy', 'copy', ['↵'])]
        : [A('Paste into window', 'clipboard', 'default', ['↵']), A('Copy only', 'copy', 'copy', ['Ctrl', '↵'])];
    default:
      return [];
  }
}

let toastTimer = 0;
function toast(msg, err = false) {
  const t = $('toast');
  t.className = `toast on ${err ? 'err' : ''}`;
  t.innerHTML = `${glyph(err ? 'warn' : 'check', 15)}${esc(msg)}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('on'), 1800);
}

async function runAction(it, act) {
  if (!it || !act) return;
  if (act.mode === 'keyword-prompt') return keywordPrompt(it);
  if (act.mode === 'media-next' || act.mode === 'media-prev') {
    it = { ...it, op: act.mode.slice(6) };
    act = { mode: 'default' };
  }
  closeMenu();
  // dangerous things ask twice: first Enter arms, second runs
  const needsConfirm = (it.confirm && act.mode === 'default') || act.mode === 'recycle';
  const armKey = `${it.key}|${act.mode}`;
  if (needsConfirm && S.confirmKey !== armKey && S.confirmKey !== it.key) {
    S.confirmKey = act.mode === 'default' ? it.key : armKey;
    renderResults(qEl.value);
    if (act.mode === 'recycle') toast('Press Shift+Delete again to move it to the Recycle Bin', true);
    return;
  }
  S.confirmKey = null;
  try {
    const r = await call('activate', { item: it, mode: act.mode, q: qEl.value });
    if (r?.toast) toast(r.toast);
    if (r?.hide) {
      hide();
    } else if (r?.refresh) {
      S.detailsFor = null;
      search(true);
      // the player takes a moment to report its new state
      if (it.kind === 'media') setTimeout(() => search(true), 450);
    }
  } catch (e) {
    if (String(e) !== 'Cancelled') toast(String(e), true);
    search(true);
  }
}

function activate(mode = 'default') {
  const it = S.flat[S.sel];
  if (!it) return;
  if (it.kind === 'hint') return;
  const acts = actionsFor(it);
  const act = acts.find((a) => a.mode === mode) || (mode === 'default' ? acts[0] : null);
  if (!act) return;
  runAction(it, act);
}

// ------------------------------------------------------------------ action menu
function openMenu() {
  const it = S.flat[S.sel];
  if (!it) return;
  const items = actionsFor(it);
  if (!items.length) return;
  S.menu = { items, sel: 0, item: it };
  renderMenu();
}

function keywordPrompt(it) {
  S.menu = { items: [], sel: 0, item: it, input: true };
  menuEl.hidden = false;
  menuEl.innerHTML = `<div class="menu-head">Keyword for ${esc(it.title)}</div>
    <div class="kw-box">${glyph('tag', 16)}<input id="kwInput" placeholder="e.g. ${esc((it.title || '').split(/\s+/)[0].toLowerCase().slice(0, 6) || 'word')}" spellcheck="false" maxlength="24" /></div>
    <div class="kw-help">Type it in Glint and this comes first. <kbd>↵</kbd> save · <kbd>Esc</kbd> cancel</div>`;
  const inp = $('kwInput');
  inp.focus();
  inp.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); closeMenu(); }
    if (e.key === 'Enter') {
      e.preventDefault();
      const w = inp.value.trim();
      if (!w) return closeMenu();
      runAction(it, { mode: `keyword:${w}` });
    }
  };
}

function closeMenu() {
  S.menu = null;
  menuEl.hidden = true;
  qEl.focus();
}

function renderMenu() {
  const m = S.menu;
  menuEl.hidden = false;
  menuEl.innerHTML = `<div class="menu-head">${esc(m.item.title)}</div><div class="menu-list">${m.items
    .map((a, i) => `<div class="mi ${i === m.sel ? 'sel' : ''} ${a.danger ? 'danger' : ''}" data-i="${i}">${glyph(a.icon, 16)}<span>${esc(a.label)}</span>${a.keys?.length ? `<span class="keys">${a.keys.map((k) => `<kbd>${k}</kbd>`).join('')}</span>` : ''}</div>`)
    .join('')}</div>`;
  menuEl.querySelectorAll('.mi').forEach((el) => {
    el.onmousemove = () => { if (m.sel !== +el.dataset.i) { m.sel = +el.dataset.i; renderMenu(); } };
    el.onclick = () => runAction(m.item, m.items[+el.dataset.i]);
  });
  menuEl.querySelector('.mi.sel')?.scrollIntoView({ block: 'nearest' });
}

// ------------------------------------------------------------------ footer
function renderFooter() {
  const st = S.status;
  let status = '';
  if (st) {
    const idx = st.indexer;
    if (idx && idx.volumes?.length) {
      const ready = idx.volumes.every((v) => v.ready);
      const n = fmtCount(idx.total);
      status = ready
        ? `<span class="dot"></span>${n} files indexed${S.ms != null && qEl.value ? ` · ${S.ms} ms` : ''}`
        : `<span class="dot warn"></span>Indexing drives… ${n} so far`;
    } else if (st.indexer_installed) {
      status = `<span class="dot warn"></span>File indexer not running · <a data-act="indexer">Start it</a>`;
    } else {
      status = `<span class="dot warn"></span>Searching your user folder only`;
    }
    if (st.update) status += ` · <a data-act="update">Update to ${esc(st.update)}</a>`;
  }
  $('status').innerHTML = status;
  $('status').querySelectorAll('a').forEach((a) => (a.onclick = async () => {
    if (a.dataset.act === 'indexer') {
      try { await call('install_indexer'); toast('Starting the indexer…'); setTimeout(refreshStatus, 2500); } catch (e) { toast(String(e), true); }
    } else call('open_settings', { section: 'about' });
  }));

  const it = S.flat[S.sel];
  let hints = [];
  if (S.menu) hints = [['↑↓', 'Choose'], ['↵', 'Run'], ['Esc', 'Back']];
  else if (it) {
    const acts = actionsFor(it);
    hints.push(['↵', goLabel(it) || 'Open']);
    const second = acts.find((a) => a.mode === 'reveal') || acts.find((a) => a.mode === 'admin');
    if (second && second.keys?.length) hints.push([second.keys.join('+'), second.label.replace('Open file location', 'Location').replace('Show in folder', 'Folder')]);
    if (acts.length > 1) hints.push(['→', 'Actions']);
    if (S.mode === 'path' && it.complete) hints.unshift(['Tab', 'Complete']);
  } else hints = [['Esc', 'Close']];
  $('hints').innerHTML = hints.map(([k, l]) => `<span class="hint">${k.split('+').map((x) => `<kbd>${x}</kbd>`).join('')}${esc(l)}</span>`).join('');
}

async function refreshStatus() {
  try {
    S.status = await call('status');
    S.vscode = !!S.status.vscode;
    renderFooter();
  } catch {}
}

// ------------------------------------------------------------------ keyboard
function hide() {
  closeMenu();
  call('hide');
}

qEl.addEventListener('input', () => {
  S.historyIdx = -1;
  search();
});

document.addEventListener('keydown', (e) => {
  const k = e.key;
  const ctrl = e.ctrlKey || e.metaKey;

  // the action menu has its own keys
  if (S.menu) {
    const m = S.menu;
    if (k === 'ArrowDown') { m.sel = (m.sel + 1) % m.items.length; renderMenu(); }
    else if (k === 'ArrowUp') { m.sel = (m.sel - 1 + m.items.length) % m.items.length; renderMenu(); }
    else if (k === 'Enter') runAction(m.item, m.items[m.sel]);
    else if (k === 'Escape' || k === 'ArrowLeft' || (ctrl && k.toLowerCase() === 'k')) closeMenu();
    else return;
    e.preventDefault();
    renderFooter();
    return;
  }

  if (k === 'Escape') {
    e.preventDefault();
    if (S.confirmKey) { S.confirmKey = null; renderResults(qEl.value); return; }
    if (S.filter !== 'all') { setFilter('all'); return; }
    if (qEl.value) { qEl.value = ''; search(true); return; }
    hide();
    return;
  }
  if (k === 'ArrowDown') { e.preventDefault(); updateSel(S.sel + 1); return; }
  if (k === 'ArrowUp') {
    e.preventDefault();
    // Up on an empty box (or while browsing past queries) recalls earlier searches
    if ((!qEl.value || S.historyIdx >= 0) && S.queryHistory.length) {
      S.historyIdx = Math.min(S.historyIdx + 1, S.queryHistory.length - 1);
      qEl.value = S.queryHistory[S.historyIdx];
      search(true);
      return;
    }
    updateSel(S.sel - 1);
    return;
  }
  if (k === 'PageDown') { e.preventDefault(); updateSel(Math.min(S.sel + 6, S.flat.length - 1)); return; }
  if (k === 'PageUp') { e.preventDefault(); updateSel(Math.max(S.sel - 6, 0)); return; }
  if (k === 'Tab') {
    e.preventDefault();
    const it = S.flat[S.sel];
    if (S.mode === 'path' && it?.complete && !e.shiftKey) {
      qEl.value = it.complete;
      search(true);
      return;
    }
    cycleFilter(e.shiftKey ? -1 : 1);
    return;
  }
  if (k === 'Enter') {
    e.preventDefault();
    if (e.altKey) activate('properties');
    else if (ctrl) activate(S.flat[S.sel]?.kind === 'clip' ? 'copy' : 'reveal');
    else if (e.shiftKey) activate('admin');
    else activate('default');
    return;
  }
  if (k === 'ArrowRight' && qEl.selectionStart === qEl.value.length && S.flat[S.sel]) { e.preventDefault(); openMenu(); renderFooter(); return; }
  if (ctrl && k.toLowerCase() === 'k') { e.preventDefault(); openMenu(); renderFooter(); return; }
  if (ctrl && k === ',') { e.preventDefault(); call('open_settings', {}); return; }
  if (ctrl && !e.shiftKey && k.toLowerCase() === 'p') {
    e.preventDefault();
    S.settings.show_preview = !S.settings.show_preview;
    shellEl.classList.toggle('no-preview', !S.settings.show_preview);
    call('settings_set', { show_preview: S.settings.show_preview });
    updatePreview();
    return;
  }
  if (ctrl && /^[1-9]$/.test(k)) {
    e.preventDefault();
    const i = +k - 1;
    if (i < S.flat.length) { S.sel = i; activate('default'); }
    return;
  }
  const noTextSel = qEl.selectionStart === qEl.selectionEnd;
  if (ctrl && k.toLowerCase() === 'c' && noTextSel && S.flat[S.sel]) {
    e.preventDefault();
    const it = S.flat[S.sel];
    if (e.shiftKey) activate('copy-file');
    else if (it.kind === 'calc' || it.kind === 'color' || it.kind === 'info') activate('default');
    else if (it.kind === 'clip') activate('copy');
    else if (it.path || it.lnk) activate('copy-path');
    return;
  }
  if (ctrl && e.shiftKey && k.toLowerCase() === 'p' && S.flat[S.sel]) {
    e.preventDefault();
    const it = S.flat[S.sel];
    if (PINNABLE.includes(it.kind)) activate(actionsFor(it).find((a) => a.mode === 'pin' || a.mode === 'unpin')?.mode);
    return;
  }
  if (ctrl && k.toLowerCase() === 'o') { e.preventDefault(); activate('open-with'); return; }
  if (ctrl && k.toLowerCase() === 't') { e.preventDefault(); activate('terminal'); return; }
  if (ctrl && k.toLowerCase() === 'w') { e.preventDefault(); activate('close'); return; }
  if (k === 'Delete' && ctrl && S.flat[S.sel]?.recent) { e.preventDefault(); activate('forget'); return; }
  if (k === 'Delete' && e.shiftKey && S.flat[S.sel]) { e.preventDefault(); activate('recycle'); return; }
  if (document.activeElement !== qEl && !ctrl && k.length === 1) qEl.focus();
});

// mouse: hovering selects (only after a real mouse move, so typing doesn't jump), click opens
resultsEl.addEventListener('mousemove', (e) => {
  const row = e.target.closest('.row');
  if (row && +row.dataset.i !== S.sel) updateSel(+row.dataset.i);
});
resultsEl.addEventListener('click', (e) => {
  const row = e.target.closest('.row');
  if (!row) return;
  S.sel = +row.dataset.i;
  activate(e.ctrlKey ? 'reveal' : 'default');
});
resultsEl.addEventListener('contextmenu', (e) => {
  const row = e.target.closest('.row');
  e.preventDefault();
  if (!row) return;
  updateSel(+row.dataset.i);
  openMenu();
  renderFooter();
});
document.addEventListener('mousedown', (e) => {
  if (S.menu && !menuEl.contains(e.target)) closeMenu();
});
document.addEventListener('contextmenu', (e) => e.preventDefault());

// ------------------------------------------------------------------ show / hide
listen('launcher-shown', async () => {
  shellEl.classList.remove('again');
  void shellEl.offsetWidth;
  shellEl.classList.add('again');
  closeMenu();
  // a recent query stays (selected, so typing replaces it); an old one is cleared
  if (Date.now() - S.hiddenAt > 45000) {
    qEl.value = '';
    if (S.filter !== 'all') { S.filter = 'all'; renderTabs(); }
  }
  qEl.focus();
  qEl.select();
  S.historyIdx = -1;
  search(true);
  refreshStatus().then(renderTabs);
  call('queries').then((q) => (S.queryHistory = q || [])).catch(() => {});
});
listen('launcher-hidden', () => {
  previewEl.querySelectorAll('video,audio').forEach((m) => m.pause());
  S.hiddenAt = Date.now();
  S.confirmKey = null;
});
listen('settings-changed', (s) => {
  S.settings = s;
  applyTheme(s);
  shellEl.classList.toggle('no-preview', !s.show_preview);
  search(true);
});
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(S.settings));

(async () => {
  renderTabs();
  await loadSettings();
  refreshStatus().then(renderTabs);
  search(true);
  qEl.focus();
})();
