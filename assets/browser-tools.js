// What `shepherd browser` does in a page beyond eval: the snapshot an agent reads (an accessibility-like tree whose
// elements it can act on have refs) and the actions on them. Installed once a page, by the first command that needs
// it; the refs live as long as the page.
// ponytail: events are the page's own (untrusted), so CSS :hover doesn't apply and a page that checks isTrusted
// refuses them; the defaults a key has (Enter submits, Tab moves on) are done here for the common keys. Real input
// would go through NSEvents on the web view.
(() => {
  if (window.__shepherdTools) return;
  const ids = new WeakMap(), byRef = new Map();
  let next = 0;
  const ref = el => {
    let r = ids.get(el);
    if (!r) ids.set(el, (r = 'e' + ++next));
    byRef.set(r, new WeakRef(el));
    return r;
  };
  const norm = s => String(s ?? '').replace(/\s+/g, ' ').trim();
  const clip = (s, n = 100) => (s.length > n ? s.slice(0, n - 1) + '…' : s);
  const q = s => JSON.stringify(clip(norm(s)));
  const shown = el => {
    if (el.checkVisibility) return el.checkVisibility({ visibilityProperty: true, opacityProperty: false });
    const s = getComputedStyle(el);
    return s.display === 'contents' || (s.visibility !== 'hidden' && el.getClientRects().length > 0);
  };
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK', 'SVG', 'svg', 'PATH', 'path']);
  const INPUT = { checkbox: 'checkbox', radio: 'radio', button: 'button', submit: 'button', reset: 'button', image: 'button', range: 'slider', number: 'spinbutton', search: 'searchbox', file: 'button', color: 'button', hidden: null };
  const TAGS = {
    BUTTON: 'button', TEXTAREA: 'textbox', SUMMARY: 'button', NAV: 'navigation', MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo',
    ASIDE: 'complementary', FORM: 'form', DIALOG: 'dialog', UL: 'list', OL: 'list', LI: 'listitem', TABLE: 'table', TR: 'row',
    TD: 'cell', TH: 'columnheader', OPTION: 'option', PROGRESS: 'progressbar', IFRAME: 'iframe', H1: 'heading', H2: 'heading',
    H3: 'heading', H4: 'heading', H5: 'heading', H6: 'heading',
  };
  const role = el => {
    const r = el.getAttribute('role');
    if (r) return r.split(' ')[0];
    switch (el.tagName) {
      case 'A': return el.hasAttribute('href') ? 'link' : null;
      case 'INPUT': return (el.type in INPUT) ? INPUT[el.type] : 'textbox';
      case 'SELECT': return el.multiple || el.size > 1 ? 'listbox' : 'combobox';
      case 'IMG': return el.alt === '' ? null : 'img';
    }
    return TAGS[el.tagName] || (el.isContentEditable && !el.parentElement?.isContentEditable ? 'textbox' : null);
  };
  const ACTIVE = new Set(['link', 'button', 'textbox', 'searchbox', 'checkbox', 'radio', 'combobox', 'listbox', 'slider', 'spinbutton', 'switch', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'treeitem']);
  // what an agent reads whole: no tree under them
  const LEAF = new Set(['link', 'button', 'textbox', 'searchbox', 'checkbox', 'radio', 'combobox', 'slider', 'spinbutton', 'switch', 'img', 'heading', 'option', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'progressbar']);
  const active = (el, r) => {
    if (ACTIVE.has(r) || el.isContentEditable) return true;
    if (el.hasAttribute('onclick') || el.tabIndex >= 0 && el.hasAttribute('tabindex')) return true;
    // a div made clickable: the pointer is its own, not inherited from a clickable parent
    return getComputedStyle(el).cursor === 'pointer' && el.parentElement && getComputedStyle(el.parentElement).cursor !== 'pointer';
  };
  // the roles whose name is their text; a list or a form is named only by aria-label
  const FROM_TEXT = new Set(['link', 'button', 'heading', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'checkbox', 'radio', 'switch', 'treeitem', 'cell', 'columnheader', 'generic', 'img']);
  const name = (el, r) => {
    const own = el.getAttribute('aria-label');
    if (own) return own;
    const by = el.getAttribute('aria-labelledby');
    if (by) return by.split(/\s+/).map(id => el.ownerDocument.getElementById(id)?.innerText || '').join(' ');
    if (el.type === 'file') return `${el.labels?.length ? [...el.labels].map(l => l.innerText).join(' ') + ': ' : ''}${[...el.files].map(f => f.name).join(', ') || 'no file chosen'}`;
    if (el.labels?.length) return [...el.labels].map(l => l.innerText).join(' ');
    if (el.tagName === 'INPUT' && ['button', 'submit', 'reset'].includes(el.type)) return el.value;
    if (el.alt) return el.alt;
    if (r === 'textbox' || r === 'searchbox' || r === 'combobox' || r === 'listbox' || r === 'spinbutton' || r === 'slider') return el.title || el.placeholder || '';
    return FROM_TEXT.has(r || 'generic') ? el.innerText || el.title || el.textContent || '' : el.title || '';
  };
  const describe = el => {
    const r = role(el);
    const act = active(el, r);
    if (!r && !act) return null;
    const out = [`- ${r || 'generic'}`];
    const n = norm(name(el, r));
    if (n) out.push(q(n));
    if (/^H[1-6]$/.test(el.tagName)) out.push(`[level=${el.tagName[1]}]`);
    if (act) out.push(`[ref=${ref(el)}]`);
    if ('checked' in el && (el.type === 'checkbox' || el.type === 'radio')) el.checked && out.push('[checked]');
    else if (el.getAttribute('aria-checked') === 'true') out.push('[checked]');
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') out.push('[disabled]');
    const exp = el.getAttribute('aria-expanded');
    if (exp) out.push(exp === 'true' ? '[expanded]' : '[collapsed]');
    if (el.getAttribute('aria-selected') === 'true' || (el.tagName === 'OPTION' && el.selected)) out.push('[selected]');
    if (el === deepActive()) out.push('[focused]');
    if (el.tagName === 'A' && el.getAttribute('href')) out.push(`(${clip(el.getAttribute('href'), 80)})`);
    if (el.tagName === 'SELECT') out.push(`: ${q([...el.selectedOptions].map(o => o.text).join(', '))}`);
    else if (r === 'textbox' || r === 'searchbox' || r === 'spinbutton' || r === 'slider') {
      const v = el.isContentEditable ? el.innerText : el.type === 'password' ? '•'.repeat(el.value.length) : el.value;
      if (v) out.push(`: ${q(v)}`);
    }
    return { line: out.join(' '), leaf: LEAF.has(r), named: FROM_TEXT.has(r || 'generic') && !!n };
  };
  const kids = el => {
    if (el.tagName === 'IFRAME') {
      try { return el.contentDocument?.body ? [el.contentDocument.body] : []; } catch { return []; }
    }
    if (el.tagName === 'SLOT') return el.assignedNodes({ flatten: true });
    return [...(el.shadowRoot?.childNodes || []), ...el.childNodes];
  };
  // `quiet`: under an element named by its text, which the text would only repeat
  const walk = (el, depth, out, quiet) => {
    for (const c of kids(el)) {
      if (out.length > 3000) return;
      if (c.nodeType === 3) {
        const t = norm(c.textContent);
        if (t && !quiet) out.push({ depth, text: t });
        continue;
      }
      if (c.nodeType !== 1 || SKIP.has(c.tagName) || !shown(c)) continue;
      const d = describe(c);
      if (d) {
        out.push({ depth, line: d.line });
        if (!d.leaf || c.tagName === 'IFRAME') walk(c, depth + 1, out, quiet || d.named);
      } else {
        // a block's text is its own line, not run on with the text around it
        const block = !getComputedStyle(c).display.startsWith('inline');
        if (block) out.push({ depth, cut: true });
        walk(c, depth, out, quiet);
        if (block) out.push({ depth, cut: true });
      }
    }
  };
  function deepActive() {
    let a = document.activeElement;
    while (a?.shadowRoot?.activeElement) a = a.shadowRoot.activeElement;
    while (a?.tagName === 'IFRAME') {
      try { a = a.contentDocument.activeElement; } catch { break; }
    }
    return a;
  }
  const find = target => {
    if (/^e\d+$/.test(target)) {
      const el = byRef.get(target)?.deref();
      if (!el || !el.isConnected) throw new Error(`no element ${target} on the page now: take a new snapshot`);
      return el;
    }
    if (target.startsWith('text=')) {
      const want = norm(target.slice(5)).toLowerCase();
      // the innermost visible element whose text is it, an active one first
      let best = null;
      for (const el of document.querySelectorAll('body *')) {
        if (SKIP.has(el.tagName) || !shown(el)) continue;
        const t = norm(el.innerText || el.value || el.getAttribute('aria-label')).toLowerCase();
        if (!t.includes(want)) continue;
        const exact = t === want, act = active(el, role(el));
        const score = (exact ? 2 : 0) + (act ? 1 : 0);
        if (!best || score > best.score || (score === best.score && best.el.contains(el))) best = { el, score };
      }
      if (!best) throw new Error(`no element with the text ${JSON.stringify(target.slice(5))}`);
      return best.el;
    }
    let el;
    try { el = document.querySelector(target); } catch { throw new Error(`${target}: not a ref (e12), text=… or a CSS selector`); }
    if (!el) throw new Error(`nothing matches ${target}`);
    return el;
  };
  // what an answer calls the element: its role and name, taken before the action changes them
  const label = el => {
    const r = role(el), n = clip(norm(name(el, r)), 60);
    return `${r || el.tagName.toLowerCase()}${n ? ' ' + JSON.stringify(n) : ''}`;
  };
  const center = el => {
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    return { clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
  };
  const fire = (el, type, at, more) => {
    const Kind = type.startsWith('pointer') ? PointerEvent : MouseEvent;
    return el.dispatchEvent(new Kind(type, { bubbles: !type.endsWith('enter') && !type.endsWith('leave'), cancelable: true, composed: true, view: window, button: 0, buttons: type.endsWith('down') ? 1 : 0, ...at, ...more }));
  };
  const usable = el => {
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') throw new Error(`${label(el)} is disabled`);
  };
  // what else is under the point, when it isn't the element: a modal or an overlay takes the click in a real one
  const covered = (el, at) => {
    const hit = el.ownerDocument.elementFromPoint(at.clientX, at.clientY);
    return hit && hit !== el && !el.contains(hit) && !hit.contains(el) ? ` (note: ${label(hit)} is over it)` : '';
  };
  const hover = el => {
    const at = center(el);
    for (const t of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'pointermove', 'mousemove']) fire(el, t, at);
    return at;
  };
  const click = (el, count = 1) => {
    usable(el);
    const at = hover(el), note = covered(el, at);
    for (let i = 1; i <= count; i++) {
      fire(el, 'pointerdown', at, { detail: i });
      fire(el, 'mousedown', at, { detail: i });
      if (i === 1 && el.focus) el.focus({ preventScroll: true });
      fire(el, 'pointerup', at, { detail: i });
      fire(el, 'mouseup', at, { detail: i });
      // the element's own click, so a link, a checkbox or a submit button does what it does
      el.click();
    }
    if (count === 2) fire(el, 'dblclick', at, { detail: 2 });
    return note;
  };
  const editable = el => el.isContentEditable || el.tagName === 'TEXTAREA' || (el.tagName === 'INPUT' && !(el.type in INPUT && INPUT[el.type] !== 'searchbox' && INPUT[el.type] !== 'spinbutton' && INPUT[el.type] !== 'slider'));
  // text typed in where the caret is, as the page's own input events see it; the setter when execCommand can't
  const insert = (el, text) => {
    if (text && el.ownerDocument.execCommand('insertText', false, text)) return;
    if (!text && el.ownerDocument.execCommand('delete')) return;
    if (el.isContentEditable) { el.textContent += text; }
    else {
      const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text ? el.value + text : '');
    }
    el.dispatchEvent(new InputEvent('input', { bubbles: true, composed: true, inputType: text ? 'insertText' : 'deleteContent', data: text || null }));
  };
  const selectAll = el => {
    el.focus({ preventScroll: true });
    if (el.isContentEditable) el.ownerDocument.execCommand('selectAll');
    else el.select?.();
  };
  const KEYS = { enter: 'Enter', tab: 'Tab', escape: 'Escape', esc: 'Escape', backspace: 'Backspace', delete: 'Delete', space: ' ', arrowup: 'ArrowUp', arrowdown: 'ArrowDown', arrowleft: 'ArrowLeft', arrowright: 'ArrowRight', up: 'ArrowUp', down: 'ArrowDown', left: 'ArrowLeft', right: 'ArrowRight', home: 'Home', end: 'End', pageup: 'PageUp', pagedown: 'PageDown' };
  const focusables = () => [...document.querySelectorAll('a[href], button, input, select, textarea, summary, [tabindex], [contenteditable=""], [contenteditable="true"]')].filter(e => e.tabIndex >= 0 && !e.disabled && shown(e));
  const press = combo => {
    // `+` alone is the key, not a joint
    const parts = combo.length > 1 ? combo.split('+') : [combo];
    const raw = parts.pop() || '+';
    const mods = new Set(parts.map(p => p.toLowerCase()));
    const key = KEYS[raw.toLowerCase()] || raw;
    const el = deepActive() || document.body, l = label(el);
    const opts = { key, code: key.length === 1 ? (/[a-z]/i.test(key) ? 'Key' + key.toUpperCase() : /\d/.test(key) ? 'Digit' + key : '') : key, bubbles: true, cancelable: true, composed: true, metaKey: mods.has('meta') || mods.has('cmd'), ctrlKey: mods.has('control') || mods.has('ctrl'), altKey: mods.has('alt') || mods.has('option'), shiftKey: mods.has('shift') };
    const go = el.dispatchEvent(new KeyboardEvent('keydown', opts));
    if (go && key.length === 1 && !opts.metaKey && !opts.ctrlKey) el.dispatchEvent(new KeyboardEvent('keypress', opts));
    let did = '';
    if (go) {
      const cmd = opts.metaKey || opts.ctrlKey;
      if (cmd && key.toLowerCase() === 'a') { selectAll(el); did = 'selected all'; }
      else if (key === 'Enter') {
        if (el.tagName === 'TEXTAREA' || el.isContentEditable) insert(el, '\n');
        else if (el.tagName === 'INPUT' && el.form) { el.form.requestSubmit(); did = 'submitted the form'; }
        else if (/^(A|BUTTON|SUMMARY)$/.test(el.tagName) || role(el) === 'button' || role(el) === 'link') { el.click(); did = 'clicked it'; }
      } else if (key === ' ' && (el.tagName === 'BUTTON' || el.type === 'checkbox' || el.type === 'radio' || role(el) === 'button')) { el.click(); did = 'clicked it'; }
      else if (key === 'Tab') {
        const list = focusables(), at = list.indexOf(el);
        const to = list[(at + (opts.shiftKey ? -1 : 1) + list.length) % list.length];
        to?.focus();
        did = to ? `focus on ${label(to)}` : '';
      } else if (key === 'Backspace' && editable(el)) el.ownerDocument.execCommand('delete');
      else if (key === 'Delete' && editable(el)) el.ownerDocument.execCommand('forwardDelete');
      else if (key.length === 1 && !cmd && editable(el)) insert(el, key);
    }
    el.dispatchEvent(new KeyboardEvent('keyup', opts));
    return `pressed ${combo} on ${l}${go ? '' : ' (the page prevented its default)'}${did ? ': ' + did : ''}`;
  };
  const scroller = () => {
    const s = document.scrollingElement || document.documentElement;
    if (s.scrollHeight > s.clientHeight + 1) return s;
    // an app that scrolls a box of its own, not the page: its biggest one
    let best = null, area = 0;
    for (const el of document.querySelectorAll('body *')) {
      if (el.scrollHeight <= el.clientHeight + 1) continue;
      const o = getComputedStyle(el).overflowY;
      if (o !== 'auto' && o !== 'scroll') continue;
      const a = el.clientWidth * el.clientHeight;
      if (a > area) { best = el; area = a; }
    }
    return best || s;
  };
  const T = {
    snapshot(target) {
      const root = target ? find(target) : document.body;
      const out = [];
      if (target) {
        const d = describe(root);
        d && out.push({ depth: 0, line: d.line });
      }
      walk(root, target ? 1 : 0, out);
      // the text pieces of one line of prose (a link's or a <b>'s around them aside) are one text
      const lines = [];
      for (const o of out) {
        if (o.cut) { lines.at(-1)?.text !== undefined && lines.push({ cut: true }); continue; }
        const last = lines.at(-1);
        if (o.text !== undefined && last?.text !== undefined && last.depth === o.depth) last.text += ' ' + o.text;
        else lines.push({ ...o });
      }
      const s = scroller();
      const head = `# ${document.title} — ${location.href}\n# viewport ${innerWidth}x${innerHeight}, scrolled ${Math.round(s.scrollTop)} of ${Math.max(0, s.scrollHeight - s.clientHeight)}`;
      const body = lines.filter(l => !l.cut).map(l => '  '.repeat(l.depth) + (l.line ?? `- text: ${q(l.text.length > 300 ? l.text.slice(0, 299) + '…' : l.text)}`));
      return [head, ...body, ...(out.length > 3000 ? ['(cut at 3000 lines: `snapshot <ref>` reads one part)'] : [])].join('\n');
    },
    click: t => { const el = find(t), l = label(el); return `clicked ${l}${click(el)}`; },
    dblclick: t => { const el = find(t), l = label(el); return `double-clicked ${l}${click(el, 2)}`; },
    hover: t => { const el = find(t); hover(el); return `hovering ${label(el)}`; },
    focus: t => { const el = find(t); center(el); el.focus(); return `focused ${label(el)}`; },
    fill(t, text) {
      const el = find(t);
      usable(el);
      if (!editable(el)) throw new Error(`${label(el)} isn't a text field: \`select\` picks an option, \`check\` ticks a box`);
      center(el);
      selectAll(el);
      insert(el, text);
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return `filled ${label(el)}`;
    },
    type(t, text) {
      const el = find(t);
      usable(el);
      center(el);
      el.focus();
      if (!el.isContentEditable && el.setSelectionRange && el.value !== undefined) try { el.setSelectionRange(el.value.length, el.value.length); } catch {}
      for (const ch of text) press(ch === '\n' ? 'Enter' : ch === ' ' ? 'Space' : ch);
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return `typed into ${label(el)}`;
    },
    press,
    select(t, values) {
      const el = find(t);
      usable(el);
      if (el.tagName !== 'SELECT') throw new Error(`${label(el)} isn't a <select>: \`click\` opens a custom one, then \`click\` its option`);
      const want = values.map(v => norm(v).toLowerCase());
      const picked = [...el.options].filter(o => want.includes(o.value.toLowerCase()) || want.includes(norm(o.text).toLowerCase()));
      if (!picked.length) throw new Error(`no option ${values.join(', ')}: ${[...el.options].map(o => JSON.stringify(o.text)).join(', ')}`);
      for (const o of el.options) o.selected = picked.includes(o) && (el.multiple || o === picked[0]);
      el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return `selected ${picked.map(o => JSON.stringify(o.text)).join(', ')} in ${label(el)}`;
    },
    check(t, on) {
      const el = find(t);
      const l = label(el);
      const is = () => ('checked' in el && (el.type === 'checkbox' || el.type === 'radio')) ? el.checked : el.getAttribute('aria-checked') === 'true';
      if (is() !== on) click(el);
      if (is() !== on) throw new Error(`${l} is still ${on ? 'unchecked' : 'checked'} after a click`);
      return `${on ? 'checked' : 'unchecked'} ${l}`;
    },
    scroll(to) {
      const s = scroller();
      const page = s.clientHeight * 0.85;
      const by = { down: page, up: -page, top: -s.scrollHeight, bottom: s.scrollHeight }[to];
      if (by === undefined) { const el = find(to); center(el); return `scrolled to ${label(el)}`; }
      s.scrollBy({ top: by, behavior: 'instant' });
      return `scrolled ${to}: ${Math.round(s.scrollTop)} of ${Math.max(0, s.scrollHeight - s.clientHeight)}`;
    },
    upload(t, files) {
      const el = find(t);
      if (el.tagName !== 'INPUT' || el.type !== 'file') throw new Error(`${label(el)} isn't a file input (input[type=file]): a custom button usually has one hidden next to it`);
      const dt = new DataTransfer();
      for (const f of files) dt.items.add(new File([Uint8Array.from(atob(f.data), c => c.charCodeAt(0))], f.name, { type: f.type }));
      el.files = dt.files;
      el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return `put ${files.map(f => f.name).join(', ')} in ${label(el)}`;
    },
    // where the element is, in the page's CSS pixels, once scrolled to
    rect(t) {
      const el = find(t);
      el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      const r = el.getBoundingClientRect();
      return JSON.stringify({ x: r.left, y: r.top, w: r.width, h: r.height, vw: innerWidth, vh: innerHeight });
    },
    // the page's scroll, set to `top` first when given, for a shot of the whole page
    page(top) {
      const s = document.scrollingElement || document.documentElement;
      if (top !== undefined) s.scrollTop = top;
      return JSON.stringify({ top: s.scrollTop, height: s.scrollHeight, vw: innerWidth, vh: innerHeight });
    },
    // fixed and sticky elements hidden (`on`) or back: each piece of a whole-page shot would repeat them
    fixed(on) {
      if (on) {
        for (const el of document.querySelectorAll('body *')) {
          const p = getComputedStyle(el).position;
          if ((p === 'fixed' || p === 'sticky') && shown(el)) { el.dataset.shepherdHidden = el.style.visibility; el.style.visibility = 'hidden'; }
        }
      } else {
        for (const el of document.querySelectorAll('[data-shepherd-hidden]')) { el.style.visibility = el.dataset.shepherdHidden; delete el.dataset.shepherdHidden; }
      }
      return 'ok';
    },
    seen(t) { try { const el = find(t); return shown(el); } catch { return false; } },
  };
  Object.defineProperty(window, '__shepherdTools', { value: T });
})();
