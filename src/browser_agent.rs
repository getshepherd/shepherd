//! Agents in panes drive Shepherd's browser: `shepherd browser …` lists the browser tabs (the right panel's and the
//! sidebar's pages), runs JavaScript in one, reads its requests and console, loads, reloads and shoots it. The CLI is
//! another process, so it talks to the running window over a unix socket (`socket`): one JSON line asked, one
//! answered. The window's side of each command is `shell::browser_agent`; what records requests and console lines in
//! the page is `INSTRUMENT`, a script every page gets before its own run.
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// What the CLI asks: its arguments, the workspace of the pane it runs in, and its directory (a screenshot's path is
/// relative to it).
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub args: Vec<String>,
    pub workspace: Option<String>,
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
    Screenshot(Option<PathBuf>),
}

/// The arguments after `browser`: `--tab <id>` anywhere, then the command.
pub fn parse(args: &[String]) -> Result<(Option<String>, Command), String> {
    let mut tab = None;
    let mut timeout = Duration::from_secs(30);
    let mut clear = false;
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
            _ => rest.push(a.clone()),
        }
    }
    let one = |what: &str| rest.get(1).cloned().ok_or(format!("`{}` needs {what}", rest[0]));
    let command = match rest.first().map(String::as_str) {
        Some("tabs") => Command::Tabs,
        Some("eval") => Command::Eval { code: rest[1..].join(" "), timeout },
        Some("network") => Command::Network { entry: rest.get(1).map(|n| n.trim_start_matches('#').parse().map_err(|_| format!("{n}: not a request's number"))).transpose()?, clear },
        Some("console") => Command::Console { clear },
        Some("open") => Command::Open(one("a URL")?),
        Some("new") => Command::New(one("a URL")?),
        Some("reload") => Command::Reload,
        Some("screenshot") => Command::Screenshot(rest.get(1).map(PathBuf::from)),
        Some(other) => return Err(format!("no command {other}\n\n{HELP}")),
        None => return Err(HELP.into()),
    };
    if matches!(&command, Command::Eval { code, .. } if code.trim().is_empty()) {
        return Err("`eval` needs code: `shepherd browser eval 'document.title'`, or `-` to read it from stdin".into());
    }
    Ok((tab, command))
}

pub const HELP: &str = "Drive Shepherd's browser: the browser tabs in its right panel and the pages in its sidebar.

Usage: shepherd browser [--tab <id>] <command> [arguments]

Without --tab a command goes to the browser tab shown in this workspace's right panel, else to the workspace's
last one.

  tabs                     Every tab and page: id, workspace, url, title, whether shown, whether this workspace's.
  eval <code>              Runs JavaScript in the page and prints the result: a string as is, anything else as JSON.
                           An expression, or statements that `return` the value; `await` works, a Promise is awaited.
                           `-` reads the code from stdin. --timeout <seconds> (30).
  network [<n>]            The page's requests: #n, method, status, kind, time, URL. fetch and XMLHttpRequest
                           ones (#n) have their status and, for text, their body: `network <n>` prints it.
                           Others (images, scripts, CSS) come from Resource Timing. --clear empties the list.
  console                  The page's console lines and uncaught errors since it loaded. --clear empties them.
  open <url>               Loads the URL in the tab.
  new <url>                A new browser tab in this workspace, on the URL; prints its id.
  reload                   Reloads the tab.
  screenshot [<path>]      The page as a PNG (macOS); prints the path.

Requests and console lines are recorded from the page's load: reload a page opened earlier to see its first ones.
";

/// `shepherd browser …`: sends the arguments to the window and prints its answer.
pub fn cli(args: &[String]) -> i32 {
    if args.first().is_none_or(|a| matches!(a.as_str(), "-h" | "--help" | "help")) {
        print!("{HELP}");
        return 0;
    }
    let mut args = args.to_vec();
    // `eval -`: the code from stdin, so quotes and newlines need no escaping
    if args.iter().any(|a| a == "eval") && args.last().is_some_and(|a| a == "-") {
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
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = socket();
    if UnixStream::connect(&path).is_ok() {
        eprintln!("shepherd: {} is another Shepherd's: agents' `shepherd browser` goes to it", path.display());
        return;
    }
    // a socket left by a Shepherd that quit
    let _ = std::fs::remove_file(&path);
    let _ = path.parent().map(std::fs::create_dir_all);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return eprintln!("shepherd: {}: {e}", path.display()),
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
