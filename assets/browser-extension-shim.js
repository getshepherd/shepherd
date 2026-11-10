// Shepherd's shim for Chrome extensions. Shepherd loads it before the background service worker and the content
// scripts of an extension it unpacked from the Chrome Web Store (browser_extensions::prepare). It pins the extension's
// `chrome` and `browser`, and in the background adds the Chrome APIs WebKit doesn't have. A background written for
// Chrome calls some of them on its first lines, and one `undefined.addListener` there stops the whole script before
// it listens for its content scripts' messages, which then get no answer.
//
// Each member is added only where WebKit has nothing of that name, so one WebKit grows later is its own. A namespace
// WebKit lacks altogether is added only when the manifest asks for its permission, as Chrome gives it. What WebKit
// has was checked against its Source/WebKit/WebProcess/Extensions/Interfaces/WebExtensionAPI*.idl (2026-11).
(() => {
  // Proton Pass swaps the global chrome and browser for Proxies of its own that answer nothing, to keep the API from
  // other scripts, in its background and its content scripts alike; in Chrome that is harmless, but WebKit then
  // delivers no event, no message, to that world's listeners (a content script no longer hears its autofill).
  // Pinned, the swap fails (a TypeError in its timer) and WebKit's stay.
  for (const name of ['chrome', 'browser']) {
    try {
      if (globalThis[name]) Object.defineProperty(globalThis, name, { value: globalThis[name], writable: false, configurable: false });
    } catch {}
  }
  // the stand-ins are for the background; a content script has none of these APIs in Chrome either
  if (typeof ServiceWorkerGlobalScope === 'undefined' || !(globalThis instanceof ServiceWorkerGlobalScope)) return;
  // an event that never fires
  const event = () => ({ addListener() {}, removeListener() {}, hasListener: () => false, hasListeners: () => false });
  // a call that does nothing and answers `value` (or what `value` returns for its arguments), by promise and by
  // callback, as Chrome's calls do
  const answer = (value) => (...args) => {
    const done = typeof args[args.length - 1] === 'function' ? args.pop() : null;
    const result = Promise.resolve(typeof value === 'function' ? value(...args) : value);
    return done ? void result.then(done) : result;
  };
  // WebKit makes a namespace's object anew once nothing holds the old one, and what was added to the old one goes
  // with it (Proton Pass's long first run lost runtime.onUpdateAvailable so); holding each one filled keeps it
  const kept = [];
  Object.defineProperty(globalThis, Symbol('shepherd shim'), { value: kept });
  const fill = (target, members) => {
    if (!target) return;
    kept.push(target);
    for (const [name, value] of Object.entries(members)) {
      if (target[name] !== undefined) continue;
      try {
        Object.defineProperty(target, name, { value, configurable: true, writable: true, enumerable: true });
      } catch {}
    }
  };
  const permissions = new Set(globalThis.chrome?.runtime?.getManifest?.()?.permissions ?? []);
  // the namespace `name` of `api`, made empty for its members to be filled when WebKit lacks it and the manifest asks
  const namespace = (api, name) => {
    if (api[name] === undefined && permissions.has(name)) fill(api, { [name]: {} });
    return api[name];
  };

  for (const api of new Set([globalThis.chrome, globalThis.browser].filter(Boolean))) {
    // Vimium listens for pushState and hash changes at its top level; WebKit reports only whole navigations.
    fill(api.webNavigation, {
      onHistoryStateUpdated: event(),
      onReferenceFragmentUpdated: event(),
      onCreatedNavigationTarget: event(),
      onTabReplaced: event(),
    });
    // Proton Pass listens for updates at its top level; extensions here update only by installing them again.
    fill(api.runtime, { onUpdateAvailable: event(), onRestartRequired: event(), onSuspend: event() });
    // Vimium's omnibar completes from history; Shepherd keeps none for extensions, so there is none to find.
    fill(namespace(api, 'history'), {
      search: answer([]),
      getVisits: answer([]),
      addUrl: answer(undefined),
      deleteUrl: answer(undefined),
      deleteRange: answer(undefined),
      deleteAll: answer(undefined),
      onVisited: event(),
      onVisitRemoved: event(),
      onTitleChanged: event(),
    });
    // Vimium's omnibar completes from bookmarks; WebKit builds bookmarks only behind a flag, and Shepherd has none.
    fill(namespace(api, 'bookmarks'), {
      getTree: answer(() => [{ id: '0', title: '', children: [] }]),
      getSubTree: answer([]),
      getChildren: answer([]),
      getRecent: answer([]),
      get: answer([]),
      search: answer([]),
      onCreated: event(),
      onRemoved: event(),
      onChanged: event(),
      onMoved: event(),
    });
    // Vimium searches what's typed in its omnibar; it opens as Shepherd's address bar searches (DuckDuckGo).
    fill(namespace(api, 'search'), {
      query: answer(({ text = '', disposition, tabId } = {}) => {
        const url = 'https://duckduckgo.com/?q=' + encodeURIComponent(text);
        const opened = disposition === 'NEW_TAB' || disposition === 'NEW_WINDOW'
          ? api.tabs.create({ url })
          : tabId === undefined ? api.tabs.update({ url }) : api.tabs.update(tabId, { url });
        return opened.then(() => undefined);
      }),
    });
    // Vimium reopens closed tabs; Shepherd keeps no closed ones for extensions, so there is nothing to restore.
    fill(namespace(api, 'sessions'), {
      MAX_SESSION_RESULTS: 25,
      getRecentlyClosed: answer([]),
      getDevices: answer([]),
      restore: answer(undefined),
      onChanged: event(),
    });
    // Vimium tells of its upgrade by a notification; WebKit builds notifications only behind a flag, so none shows.
    fill(namespace(api, 'notifications'), {
      create: answer((id) => (typeof id === 'string' ? id : crypto.randomUUID())),
      update: answer(false),
      clear: answer(false),
      getAll: answer({}),
      getPermissionLevel: answer('denied'),
      onClicked: event(),
      onClosed: event(),
      onButtonClicked: event(),
    });
  }
})();
