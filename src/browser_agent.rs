//! Agents in panes drive Shepherd's browser: `shepherd browser …` lists the browser tabs (the right panel's and the
//! sidebar's pages), runs JavaScript in one, reads its requests and console, loads, reloads and shoots it. The CLI is
//! another process, so it talks to the running window over a unix socket (`socket`): one JSON line asked, one
//! answered. The window's side of each command is `shell::browser_agent`; what records requests and console lines in
//! the page is `INSTRUMENT`, a script every page gets before its own run.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// What the CLI asks: its arguments, the workspace of the pane it runs in, and its directory (a screenshot's path is
/// relative to it).
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub args: Vec<String>,
    pub workspace: Option<String>,
    /// The agent's pane, which the tab names while the agent drives it.
    #[serde(default)]
    pub pane: Option<String>,
    pub cwd: PathBuf,
}

/// What the window answers: printed to stdout when `ok`, else to stderr.
#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub out: String,
}

impl Reply {
    pub fn ok(out: impl Into<String>) -> Self {
        Reply { ok: true, out: out.into() }
    }
    pub fn err(out: impl Into<String>) -> Self {
        Reply { ok: false, out: out.into() }
    }
}

/// Where the window listens. `SHEPHERD_BROWSER_SOCKET` gives a second Shepherd (the mock fixture's) its own.
pub fn socket() -> PathBuf {
    std::env::var_os("SHEPHERD_BROWSER_SOCKET").map_or_else(|| crate::settings::cache_dir().join("browser.sock"), PathBuf::from)
}

/// A command, as the window runs it; `tab` is `--tab`'s, when given.
#[derive(Debug, PartialEq)]
pub enum Command {
    Tabs,
    Eval { code: String, timeout: Duration },
    /// The requests, or one with its body.
    Network { entry: Option<u64>, clear: bool },
    Console { clear: bool },
    Open(String),
    New(String),
    Reload,
    /// Closes a browser tab; a sidebar page or an extension's tab is not the agent's to close.
    Close,
    /// The page as a PNG: what shows, the whole page (`full`), or one element.
    Screenshot { path: Option<PathBuf>, full: bool, element: Option<String> },
    /// A script of the page's tools (`TOOLS`), whose answer is `{"ok", "text"}`: the snapshot and the actions.
    Act(String),
    Wait { until: Until, timeout: Duration },
    /// Files from disk, relative to the agent's directory, put in a file input.
    Upload { target: String, files: Vec<PathBuf> },
    /// The page laid out at this size in CSS pixels, scaled down to fit; `None` takes it back to the panel's.
    Viewport(Option<(f32, f32)>),
    /// Everything WebKit keeps for the page's site: cookies (HttpOnly ones too), storage, IndexedDB, caches.
    ClearStorage,
}

/// What `wait` waits for: a time, or a check that turns true.
#[derive(Debug, PartialEq)]
pub enum Until {
    Time(Duration),
    /// `script` answers `yes` once `what` holds; it is asked again until then, a navigation in between included.
    Check { what: String, script: String },
}

/// What `shepherd browser` runs in a page beyond eval: the snapshot and the actions, as `window.__shepherdTools`.
pub const TOOLS: &str = include_str!("../assets/browser-tools.js");

/// `call` (on `T`, the tools) as a script that answers `{"ok", "text"}`, an error's message as the text.
pub fn tools_call(call: &str) -> String {
    format!(
        "{TOOLS}\n;(() => {{ const T = window.__shepherdTools; try {{ return JSON.stringify({{ ok: true, text: String({call}) }}); }} \
         catch (e) {{ return JSON.stringify({{ ok: false, text: String(e && e.message || e) }}); }} }})()"
    )
}

/// `check`, a JS expression on `T`, as a script that answers `yes` or `no`, never throwing.
fn check_script(check: &str) -> String {
    format!("{TOOLS}\n;(() => {{ const T = window.__shepherdTools; try {{ return ({check}) ? 'yes' : 'no'; }} catch {{ return 'no'; }} }})()")
}

/// A string as a JS literal.
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The arguments after `browser`: `--tab <id>` anywhere, then the command.
pub fn parse(args: &[String]) -> Result<(Option<String>, Command), String> {
    let mut tab = None;
    let mut timeout = Duration::from_secs(30);
    let mut clear = false;
    let (mut full, mut element) = (false, None);
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tab" => tab = Some(it.next().ok_or("--tab needs a tab's id (`shepherd browser tabs` lists them)")?.clone()),
            "--timeout" => {
                let s = it.next().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.).ok_or("--timeout needs seconds")?;
                timeout = Duration::from_secs_f64(s);
            }
            "--clear" => clear = true,
            "--full" => full = true,
            "--element" => element = Some(it.next().ok_or("--element needs a ref (e12), text=… or a CSS selector")?.clone()),
            _ => rest.push(a.clone()),
        }
    }
    let one = |what: &str| rest.get(1).cloned().ok_or(format!("`{}` needs {what}", rest[0]));
    let target = || one("an element: a ref from `snapshot` (e12), text=… or a CSS selector");
    // the words after the target, as one text: `fill e3 hello world`
    let text = |what: &str| (rest.len() > 2).then(|| rest[2..].join(" ")).ok_or(format!("`{}` needs an element and {what}", rest[0]));
    let act = |call: String| Command::Act(tools_call(&call));
    let command = match rest.first().map(String::as_str) {
        Some("tabs") => Command::Tabs,
        Some("eval") => Command::Eval { code: rest[1..].join(" "), timeout },
        Some("network") => Command::Network { entry: rest.get(1).map(|n| n.trim_start_matches('#').parse().map_err(|_| format!("{n}: not a request's number"))).transpose()?, clear },
        Some("console") => Command::Console { clear },
        Some("open") => Command::Open(one("a URL")?),
        Some("new") => Command::New(one("a URL")?),
        Some("reload") => Command::Reload,
        Some("close") => Command::Close,
        Some("screenshot") => Command::Screenshot { path: rest.get(1).map(PathBuf::from), full, element },
        Some("snapshot") => act(format!("T.snapshot({})", rest.get(1).map_or("undefined".into(), |t| js(t)))),
        Some(verb @ ("click" | "dblclick" | "hover" | "focus")) => act(format!("T.{verb}({})", js(&target()?))),
        Some(verb @ ("fill" | "type")) => {
            let t = target()?;
            act(format!("T.{verb}({}, {})", js(&t), js(&text("the text")?)))
        }
        Some("press") => act(format!("T.press({})", js(&one("a key: Enter, Tab, Escape, ArrowDown, a, Meta+a, Shift+Tab")?))),
        Some("select") => {
            let t = target()?;
            let values: Vec<String> = rest.iter().skip(2).cloned().collect();
            if values.is_empty() {
                return Err("`select` needs a <select> and the option's value or text".into());
            }
            act(format!("T.select({}, {})", js(&t), serde_json::to_string(&values).unwrap_or_default()))
        }
        Some(verb @ ("check" | "uncheck")) => act(format!("T.check({}, {})", js(&target()?), verb == "check")),
        Some("scroll") => act(format!("T.scroll({})", js(rest.get(1).map_or("down", String::as_str)))),
        Some("upload") => {
            let t = target()?;
            let files: Vec<PathBuf> = rest.iter().skip(2).map(PathBuf::from).collect();
            if files.is_empty() {
                return Err("`upload` needs a file input and the files' paths".into());
            }
            Command::Upload { target: t, files }
        }
        Some("back") => act("(history.back(), 'going back')".into()),
        Some("forward") => act("(history.forward(), 'going forward')".into()),
        Some("wait") => Command::Wait { until: until(&rest[1..])?, timeout },
        Some("viewport") => Command::Viewport(match one("a size, 390x844, or reset")?.as_str() {
            "reset" => None,
            size => {
                let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))).filter(|(w, h)| *w >= 100. && *h >= 100.).ok_or(format!("{size}: a size is WIDTHxHEIGHT in CSS pixels, 100 and up"))?;
                Some((w, h))
            }
        }),
        Some("dialog") => {
            // the answer the page's alert, confirm and prompt get from now on, in this tab until it closes
            let answer = match (rest.get(1).map(String::as_str), rest.get(2..).map(|r| r.join(" ")).filter(|t| !t.is_empty())) {
                (Some("accept"), text) => format!("{{ accept: true, text: {} }}", text.map_or("null".into(), |t| js(&t))),
                (Some("dismiss"), _) => "{ accept: false }".into(),
                (Some("ask"), _) => "null".into(),
                _ => return Err("`dialog` needs accept [text], dismiss or ask".into()),
            };
            act(format!(
                "(sessionStorage.setItem('__shepherdDialog', JSON.stringify({answer})), {})",
                js(match rest[1].as_str() {
                    "accept" => "alert, confirm and prompt are accepted from now on",
                    "dismiss" => "alert, confirm and prompt are dismissed from now on",
                    _ => "alert, confirm and prompt are the page's own again",
                })
            ))
        }
        Some("storage") => match rest.get(1).map(String::as_str) {
            Some("clear") => Command::ClearStorage,
            None => act(STORAGE.into()),
            Some(other) => return Err(format!("`storage {other}`: `storage` lists, `storage clear` clears")),
        },
        Some(other) => return Err(format!("no command {other}\n\n{HELP}")),
        None => return Err(HELP.into()),
    };
    if matches!(&command, Command::Eval { code, .. } if code.trim().is_empty()) {
        return Err("`eval` needs code: `shepherd browser eval 'document.title'`, or `-` to read it from stdin".into());
    }
    Ok((tab, command))
}

/// What `wait` is given: `<ms>`, `text <t>`, `url <part>`, `gone <element>`, `load`, `idle`, or an element to show.
fn until(args: &[String]) -> Result<Until, String> {
    let rest = || (args.len() > 1).then(|| args[1..].join(" ")).ok_or(format!("`wait {}` needs what to wait for", args[0]));
    let check = |what: String, check: String| Ok(Until::Check { what, script: check_script(&check) });
    match args.first().map(String::as_str) {
        None => Err("`wait` needs what to wait for: <ms>, text <t>, url <part>, gone <element>, load, idle, or an element".into()),
        Some(ms) if ms.parse::<u64>().is_ok() => Ok(Until::Time(Duration::from_millis(ms.parse().unwrap_or(0)))),
        Some("text") => {
            let t = rest()?;
            check(format!("the text {t:?}"), format!("document.body && document.body.innerText.includes({})", js(&t)))
        }
        Some("url") => {
            let t = rest()?;
            check(format!("a URL with {t:?}"), format!("location.href.includes({})", js(&t)))
        }
        Some("gone") => {
            let t = rest()?;
            check(format!("{t} gone"), format!("!T.seen({})", js(&t)))
        }
        Some("load") => check("the load".into(), "document.readyState === 'complete'".into()),
        // no fetch or XHR under way, and none ended in the last half second
        Some("idle") => check(
            "the network to settle".into(),
            "document.readyState === 'complete' && (() => { const n = window.__shepherdAgent?.net || []; \
             return !n.some(e => e.status === undefined && !e.error) && Date.now() - Math.max(0, ...n.map(e => e.t + (e.ms || 0))) > 500; })()"
                .into(),
        ),
        Some(_) => {
            let t = args.join(" ");
            check(t.clone(), format!("T.seen({})", js(&t)))
        }
    }
}

/// `storage`: the page's cookies (the ones its scripts see), local and session storage.
const STORAGE: &str = r#"(() => {
  const list = s => { try { return Object.keys(s).map(k => `  ${k} = ${JSON.stringify(s.getItem(k).slice(0, 200))}`).join('\n') || '  (empty)'; } catch (e) { return `  (${e.message})`; } };
  const cookies = document.cookie ? document.cookie.split('; ').map(c => '  ' + c).join('\n') : '  (none a script sees: HttpOnly ones are only cleared, by `storage clear`)';
  return `cookies:\n${cookies}\nlocalStorage:\n${list(localStorage)}\nsessionStorage:\n${list(sessionStorage)}`;
})()"#;

/// `shepherd browser skill` and `--help`: the commands, and how an agent goes about them. `shepherd skill` only points
/// here, so its guide stays short.
pub const HELP: &str = include_str!("../assets/agent-skill/browser.md");

/// `shepherd browser …`: sends the arguments to the window and prints its answer.
pub fn cli(args: &[String]) -> i32 {
    if args.first().is_none_or(|a| matches!(a.as_str(), "-h" | "--help" | "help" | "skill")) {
        print!("{HELP}");
        return 0;
    }
    let mut args = args.to_vec();
    // `eval -`: the code from stdin, so quotes and newlines need no escaping
    if args.iter().any(|a| matches!(a.as_str(), "eval" | "fill" | "type")) && args.last().is_some_and(|a| a == "-") {
        let mut code = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut code) {
            eprintln!("shepherd browser: stdin: {e}");
            return 1;
        }
        *args.last_mut().unwrap() = code;
    }
    let request = Request {
        args,
        workspace: std::env::var("HERDR_WORKSPACE_ID").ok().filter(|w| !w.is_empty()),
        pane: std::env::var("HERDR_PANE_ID").ok(),
        cwd: std::env::current_dir().unwrap_or_default(),
    };
    match ask(&request) {
        Ok(reply) => {
            let out = reply.out.trim_end();
            if reply.ok {
                println!("{out}");
                0
            } else {
                eprintln!("shepherd browser: {out}");
                1
            }
        }
        Err(e) => {
            eprintln!("shepherd browser: {e}");
            1
        }
    }
}

#[cfg(unix)]
fn ask(request: &Request) -> Result<Reply, String> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let path = socket();
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("Shepherd isn't running, or is older than this command ({}: {e})", path.display()))?;
    // the window bounds an eval by its own --timeout; this only keeps a stuck window from hanging the agent
    let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
    let line = serde_json::to_string(request).map_err(|e| e.to_string())? + "\n";
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).map_err(|e| format!("no answer from Shepherd: {e}"))?;
    serde_json::from_str(&answer).map_err(|e| format!("no answer from Shepherd ({e})"))
}

// ponytail: unix sockets only; Windows agents get no `shepherd browser` until it has a named pipe
#[cfg(not(unix))]
fn ask(_: &Request) -> Result<Reply, String> {
    Err("not on Windows yet".into())
}

/// A request from an agent and where its answer goes.
pub type Asked = (Request, futures::channel::oneshot::Sender<Reply>);

/// The window's side: listens on `socket()` and hands each request to `asked`, then writes back what comes on its
/// sender. Another Shepherd already answering there keeps it: this one serves nothing.
#[cfg(unix)]
pub fn serve(asked: futures::channel::mpsc::UnboundedSender<Asked>) {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = socket();
    if UnixStream::connect(&path).is_ok() {
        log::warn!("{} is another Shepherd's: agents' `shepherd browser` goes to it", path.display());
        return;
    }
    // a socket left by a Shepherd that quit
    let _ = std::fs::remove_file(&path);
    let _ = path.parent().map(std::fs::create_dir_all);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return log::warn!("{}: {e}", path.display()),
    };
    // only this user's processes may run code in their pages
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let asked = asked.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                let reply = match BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string()).and_then(|_| serde_json::from_str::<Request>(&line).map_err(|e| e.to_string())) {
                    Ok(request) => {
                        let (tx, rx) = futures::channel::oneshot::channel();
                        match asked.unbounded_send((request, tx)) {
                            Ok(()) => futures::executor::block_on(rx).unwrap_or_else(|_| Reply::err("the window closed")),
                            Err(_) => Reply::err("the window closed"),
                        }
                    }
                    Err(e) => Reply::err(format!("bad request: {e}")),
                };
                let _ = (&stream).write_all(serde_json::to_string(&reply).unwrap_or_default().as_bytes());
            });
        }
    });
}

#[cfg(not(unix))]
pub fn serve(_: futures::channel::mpsc::UnboundedSender<Asked>) {}

/// Every page's first script: keeps the page's last requests (fetch and XMLHttpRequest, with their status and a text
/// body) and console lines in `window.__shepherdAgent`, for `network` and `console`.
// ponytail: wraps the page's own fetch, XHR and console, so a page that saved them before (or a worker, or a frame)
// isn't seen; WebKit's inspector protocol would see everything, but no public API reaches it
pub const INSTRUMENT: &str = r#"(() => {
  if (window.__shepherdAgent) return;
  const A = { net: [], log: [], seq: 0 };
  Object.defineProperty(window, '__shepherdAgent', { value: A });
  const MAX = 300, BODY = 100000;
  const keep = (list, e) => { list.push(e); if (list.length > MAX) list.shift(); return e; };
  const abs = u => { try { return new URL(u, location.href).href; } catch { return String(u); } };
  const textish = t => /json|text|xml|javascript|html|urlencoded|graphql/.test(t || '') && !/event-stream/.test(t);
  const show = v => {
    if (typeof v === 'string') return v;
    if (v instanceof Error) return v.stack ? `${v}\n${v.stack}` : String(v);
    try { return JSON.stringify(v) ?? String(v); } catch { return String(v); }
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const was = console[level];
    console[level] = function (...args) {
      keep(A.log, { t: Date.now(), level, text: args.map(show).join(' ') });
      return was.apply(this, args);
    };
  }
  addEventListener('error', e => keep(A.log, { t: Date.now(), level: 'error', text: `Uncaught ${e.message} (${e.filename}:${e.lineno})` }));
  addEventListener('unhandledrejection', e => keep(A.log, { t: Date.now(), level: 'error', text: `Unhandled rejection: ${show(e.reason)}` }));
  const done = (e, status, type) => { e.status = status; e.ms = Date.now() - e.t; e.contentType = type || ''; };
  const F = window.fetch;
  if (F) window.fetch = function (input, init) {
    const e = keep(A.net, { id: ++A.seq, kind: 'fetch', method: String(init?.method || input?.method || 'GET').toUpperCase(), url: abs(input?.url ?? input), t: Date.now() });
    return F.apply(this, arguments).then(r => {
      done(e, r.status, r.headers.get('content-type'));
      // a clone tees the body: one that is huge or never ends (a stream) isn't kept
      if (textish(e.contentType) && !(+r.headers.get('content-length') > 1e6)) r.clone().text().then(t => { e.body = t.slice(0, BODY); }, () => {});
      return r;
    }, err => { e.error = String(err); e.ms = Date.now() - e.t; throw err; });
  };
  const X = XMLHttpRequest.prototype, open = X.open, send = X.send;
  X.open = function (method, url) { this.__shepherd = { method: String(method).toUpperCase(), url: abs(url) }; return open.apply(this, arguments); };
  X.send = function () {
    if (this.__shepherd) {
      const e = keep(A.net, { id: ++A.seq, kind: 'xhr', ...this.__shepherd, t: Date.now() });
      this.addEventListener('loadend', () => {
        done(e, this.status, this.getResponseHeader('content-type'));
        if (!this.status) e.error = 'failed';
        else if (textish(e.contentType) && (this.responseType === '' || this.responseType === 'text')) e.body = this.responseText.slice(0, BODY);
      });
    }
    return send.apply(this, arguments);
  };
  // alert, confirm and prompt go to the console lines, and get the answer `dialog` set, if it set one
  for (const kind of ['alert', 'confirm', 'prompt']) {
    const was = window[kind];
    window[kind] = function (message, value) {
      let answer = null;
      try { answer = JSON.parse(sessionStorage.getItem('__shepherdDialog')); } catch {}
      keep(A.log, { t: Date.now(), level: 'dialog', text: `${kind}(${show(message ?? '')})${answer ? (answer.accept ? ' accepted' : ' dismissed') : ''}` });
      if (!answer) return was.apply(this, arguments);
      if (kind === 'confirm') return answer.accept;
      if (kind === 'prompt') return answer.accept ? (answer.text ?? value ?? '') : null;
    };
  }
  try { performance.setResourceTimingBufferSize(1000); } catch {}
})();"#;

/// Reads the requests: the recorded ones (`#n`) and Resource Timing's others, by when they started; `entry`, that one
/// recorded with its body.
pub fn network_script(entry: Option<u64>, clear: bool) -> String {
    if clear {
        return "(() => { window.__shepherdAgent && (window.__shepherdAgent.net.length = 0); performance.clearResourceTimings(); return 'cleared'; })()".into();
    }
    if let Some(n) = entry {
        return format!(
            "(() => {{ const e = window.__shepherdAgent?.net.find(e => e.id === {n}); return e ? JSON.stringify(e, null, 2) : 'no request #{n}: `shepherd browser network` lists them'; }})()"
        );
    }
    r#"(() => {
  const A = window.__shepherdAgent;
  const rows = (A?.net || []).map(e => ({ t: e.t, line: `#${e.id}  ${e.method} ${e.error ? 'failed' : e.status ?? 'pending'}  ${e.kind}  ${e.ms ?? '-'}ms  ${e.url}${e.body !== undefined ? '  (body)' : ''}` }));
  const nav = performance.getEntriesByType('navigation').concat(performance.getEntriesByType('resource'));
  for (const r of nav) {
    if (r.initiatorType === 'fetch' || r.initiatorType === 'xmlhttprequest') continue;
    rows.push({ t: performance.timeOrigin + r.startTime, line: `-  GET ${r.responseStatus || ''}  ${r.entryType === 'navigation' ? 'document' : r.initiatorType}  ${Math.round(r.duration)}ms  ${r.name}` });
  }
  rows.sort((a, b) => a.t - b.t);
  return (A ? '' : '(this page loaded before Shepherd recorded requests: reload it to see fetch and XHR)\n') + rows.map(r => r.line).join('\n');
})()"#
        .into()
}

pub fn console_script(clear: bool) -> String {
    if clear {
        return "(() => { window.__shepherdAgent && (window.__shepherdAgent.log.length = 0); return 'cleared'; })()".into();
    }
    r#"(() => {
  const A = window.__shepherdAgent;
  if (!A) return '(this page loaded before Shepherd recorded its console: reload it)';
  const at = t => { const d = new Date(t); return `${d.toTimeString().slice(0, 8)}.${String(d.getMilliseconds()).padStart(3, '0')}`; };
  return A.log.map(l => `${at(l.t)} [${l.level}] ${l.text}`).join('\n');
})()"#
        .into()
}

/// Starts `code` in the page as run `id`, its outcome kept in `window.__shepherdRuns[id]` for `poll_script`. As an
/// expression when `expression`, else as a function's body; both async, so `await` works and a Promise is awaited.
/// Answers `started`, or nothing when the code doesn't parse that way. The code goes in as source, not to `eval`,
/// which a page's Content-Security-Policy may forbid.
pub fn eval_script(id: u64, code: &str, expression: bool) -> String {
    let code = code.trim();
    let body = match expression {
        true => format!("(\n{}\n)", code.trim_end_matches(';')),
        false => format!("{{\n{code}\n}}"),
    };
    // the expression's try, which didn't parse, left its SyntaxError in the console lines: it was Shepherd's, not the page's
    let unlog = match expression {
        true => "",
        false => "const L = window.__shepherdAgent?.log, last = L?.at(-1);\n  if (last?.text.startsWith('Uncaught SyntaxError') && Date.now() - last.t < 2000) L.pop();\n  ",
    };
    format!(
        r#"(() => {{
  {unlog}const R = window.__shepherdRuns || Object.defineProperty(window, '__shepherdRuns', {{ value: {{}} }}).__shepherdRuns;
  R[{id}] = null;
  const show = v => {{
    if (v === undefined) return 'undefined';
    if (typeof v === 'string') return v;
    if (typeof Element !== 'undefined' && v instanceof Element) return v.outerHTML;
    try {{ return JSON.stringify(v, null, 2) ?? String(v); }} catch {{ return String(v); }}
  }};
  (async () => {body})().then(
    v => {{ R[{id}] = {{ ok: true, text: show(v) }}; }},
    e => {{ R[{id}] = {{ ok: false, text: e && e.stack ? `${{e}}\n${{e.stack}}` : String(e) }}; }});
  return 'started';
}})()"#
    )
}

/// Run `id`'s outcome: `null` while it runs, `{"ok", "text"}` once done, `{"gone": true}` when the page navigated.
pub fn poll_script(id: u64) -> String {
    format!(
        "(() => {{ const R = window.__shepherdRuns; if (!R || !({id} in R)) return JSON.stringify({{ gone: true }}); const r = R[{id}]; if (r) delete R[{id}]; return JSON.stringify(r); }})()"
    )
}

/// A script's result as wry hands it (JSON) when it's a string; `None` for anything else, an error included.
pub fn js_string(raw: &str) -> Option<String> {
    serde_json::from_str::<String>(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse(&args("tabs")), Ok((None, Command::Tabs)));
        assert_eq!(parse(&args("--tab browser-2 eval 1 + 1")), Ok((Some("browser-2".into()), Command::Eval { code: "1 + 1".into(), timeout: Duration::from_secs(30) })));
        assert_eq!(parse(&args("network #4 --tab x")), Ok((Some("x".into()), Command::Network { entry: Some(4), clear: false })));
        assert_eq!(parse(&args("console --clear")), Ok((None, Command::Console { clear: true })));
        assert!(parse(&args("eval")).is_err());
        assert!(parse(&args("open")).is_err());
        assert_eq!(parse(&args("--tab b3 close")), Ok((Some("b3".into()), Command::Close)));
        assert_eq!(parse(&args("screenshot a.png --full")), Ok((None, Command::Screenshot { path: Some("a.png".into()), full: true, element: None })));
        assert_eq!(parse(&args("viewport 390x844")), Ok((None, Command::Viewport(Some((390., 844.))))));
        assert_eq!(parse(&args("viewport reset")), Ok((None, Command::Viewport(None))));
        assert!(parse(&args("viewport 10x10")).is_err());
        assert!(matches!(parse(&args("fill e3 hello world")), Ok((None, Command::Act(s))) if s.contains(r#"T.fill("e3", "hello world")"#)));
        assert!(matches!(parse(&args("select #size M L")), Ok((None, Command::Act(s))) if s.contains(r##"T.select("#size", ["M","L"])"##)));
        assert!(parse(&args("fill e3")).is_err());
        assert_eq!(parse(&args("wait 500")), Ok((None, Command::Wait { until: Until::Time(Duration::from_millis(500)), timeout: Duration::from_secs(30) })));
        assert!(matches!(parse(&args("wait text Saved ok")), Ok((None, Command::Wait { until: Until::Check { script, .. }, .. })) if script.contains(r#"includes("Saved ok")"#)));
        assert!(matches!(parse(&args("upload e4 a.png b.png")), Ok((None, Command::Upload { files, .. })) if files.len() == 2));
        assert!(parse(&args("dialog maybe")).is_err());
        assert!(parse(&args("fly")).is_err());
    }

    #[test]
    fn eval_wraps_code_both_ways() {
        let e = eval_script(7, "document.title;", true);
        assert!(e.contains("(async () => (\ndocument.title\n))()"), "{e}");
        let s = eval_script(7, "const a = 1; return a", false);
        assert!(s.contains("(async () => {\nconst a = 1; return a\n})()"), "{s}");
        assert_eq!(js_string("\"started\"").as_deref(), Some("started"));
        assert_eq!(js_string(""), None);
    }
}
