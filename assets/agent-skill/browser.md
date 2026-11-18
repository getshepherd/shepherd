Drive Shepherd's browser: the browser tabs in its right panel and the pages in its sidebar.

Usage: shepherd browser [--tab <id>] <command> [arguments]

`shepherd` is the binary you ran `skill` with. Without --tab a command goes to the browser tab shown in this
workspace's right panel, else to the workspace's last one; outside herdr, to the page the window shows. The user sees
which tab you drive: it is marked for a few seconds after each command.

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
  close                    Closes the tab: a browser tab only, not a sidebar page or an extension's tab. Close
                           the tabs you opened with `new` once you are done with them.
  screenshot [<path>]      The page as a PNG (macOS); prints the path.

Requests and console lines are recorded from the page's load: reload a page opened earlier to see its first ones.

The pages are the user's, signed in to their accounts: read and test freely, and ask before anything that changes
data on a site that isn't the app you are building.
