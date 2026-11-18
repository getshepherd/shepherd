Drive Shepherd's browser: the browser tabs in its right panel and the pages in its sidebar.

Usage: shepherd browser [--tab <id>] <command> [arguments]

`shepherd` is the binary you ran `skill` with. Without --tab a command goes to the browser tab shown in this
workspace's right panel, else to the workspace's last one; outside herdr, to the page the window shows. The user sees
which tab you drive: it is marked for a few seconds after each command.

Tabs
  tabs                     Every tab and page: id, workspace, url, title, whether shown, whether this workspace's.
  new <url>                A new browser tab in this workspace, on the URL; prints its id.
  open <url>               Loads the URL in the tab.
  back, forward            Through the tab's history.
  reload                   Reloads the tab.
  close                    Closes the tab: a browser tab only, not a sidebar page or an extension's tab. Close
                           the tabs you opened with `new` once you are done with them.

Reading the page
  snapshot [<element>]     The page as a tree of roles and names (links, buttons, fields, headings, text), each
                           element you can act on with a ref: `- button "Save" [ref=e12]`. With an element, only
                           its part. Take one before acting, and again after the page changes.
  eval <code>              Runs JavaScript in the page and prints the result: a string as is, anything else as JSON.
                           An expression, or statements that `return` the value; `await` works, a Promise is awaited.
                           `-` reads the code from stdin. --timeout <seconds> (30).
  network [<n>]            The page's requests: #n, method, status, kind, time, URL. fetch and XMLHttpRequest
                           ones (#n) have their status and, for text, their body: `network <n>` prints it.
                           Others (images, scripts, CSS) come from Resource Timing. --clear empties the list.
  console                  The page's console lines, uncaught errors and dialogs since it loaded. --clear empties
                           them.
  storage                  The cookies scripts see, local and session storage.
  screenshot [<path>]      The page as a PNG (macOS); prints the path. --full: the whole page, stitched as it
                           scrolls. --element <element>: that element.

Acting on it (an <element> is a ref from `snapshot`, e12; text=Sign in, the element with that text; or a CSS
selector)
  click, dblclick, hover, focus <element>
  fill <element> <text>    Replaces a field's text (an input, a textarea, a contenteditable). `-` reads it from stdin.
  type <element> <text>    Types at the end of a field's text, key by key, for fields that react to each key.
  press <key>              A key on the focused element: Enter (submits a form), Tab, Escape, ArrowDown, Backspace,
                           a, Meta+a, Shift+Tab.
  select <element> <option>...  Picks a <select>'s option(s), by value or text.
  check, uncheck <element> A checkbox, a radio or a switch.
  scroll [down|up|top|bottom|<element>]
  upload <element> <file>...  Puts files (paths relative to you) in a file input.
  dialog accept [<text>] | dismiss | ask
                           How the page's alert, confirm and prompt are answered from now on in this tab (a
                           prompt gets the text); ask gives them back to the page. They show in `console`.

Waiting
  wait <element> | text <text> | url <part> | gone <element> | load | idle | <ms>
                           Until the element shows, the text or a URL with that part is there, the element is gone,
                           the page has loaded, the network has settled (no fetch or XHR for half a second), or
                           for that long. --timeout <seconds> (30).

Setting the page up
  viewport <width>x<height> | reset
                           Lays the page out at that size in CSS pixels (390x844 a phone, 1440x900 a laptop),
                           scaled down to fit the panel; reset gives it the panel's size back.
  storage clear            Everything kept for the page's site: cookies (HttpOnly ones too), storage, IndexedDB,
                           service workers, caches. Reload after it to start afresh.

Requests and console lines are recorded from the page's load: reload a page opened earlier to see its first ones.
Actions are the page's own events: they do what a click or a key does for nearly every page, but CSS :hover doesn't
apply and a page that checks `event.isTrusted` ignores them.

The pages are the user's, signed in to their accounts: read and test freely, and ask before anything that changes
data on a site that isn't the app you are building.
