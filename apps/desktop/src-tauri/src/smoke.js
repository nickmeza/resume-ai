// Development check run by RESUME_AI_SMOKE=1 (debug builds only): reports what the UI rendered,
// then opens a plain terminal tab and reports its output.
(async () => {
  const log = (m) => window.__TAURI__.core.invoke('ui_log', { level: 'smoke', message: m });
  try {
    const items = document.querySelectorAll('#list .item[data-key]').length;
    const groups = [...document.querySelectorAll('#list .group-header .name')].slice(0, 4).map((e) => e.textContent);
    log(`lang=${window.i18n.lang} sessions=${items} groups=${JSON.stringify(groups)}`);
    log(`status="${document.querySelector('#status').textContent}" placeholder="${document.querySelector('#search').placeholder}"`);
    log(`empty="${document.querySelector('#empty p').textContent}" filter="${document.querySelector('#btn-filter').title}"`);
    const origins = [...document.querySelectorAll('#list .origin')].slice(0, 3).map((e) => e.textContent);
    log(`origin tags=${JSON.stringify(origins)}`);
    // Open a shell tab and read what the terminal shows
    await openShell();
    await new Promise((r) => setTimeout(r, 3000));
    const entry = state.terms.get(state.activeKey);
    const buf = entry.term.buffer.active;
    const lines = [];
    for (let i = 0; i < Math.min(buf.length, 6); i++) lines.push(buf.getLine(i).translateToString(true));
    log(`tab="${entry.info.title}" alive=${entry.alive} header="${document.querySelector('#term-name').textContent}"`);
    log(`terminal: ${JSON.stringify(lines.filter(Boolean))}`);
    window.api.writePty(state.activeKey, 'echo smoke-ok\r');
    await new Promise((r) => setTimeout(r, 1500));
    const all = [];
    for (let i = 0; i < buf.length; i++) all.push(buf.getLine(i).translateToString(true));
    log(`typed command echoed: ${all.join('\n').includes('smoke-ok')}`);
  } catch (e) {
    log(`FAILED: ${e && e.stack ? e.stack : e}`);
  }
})();
