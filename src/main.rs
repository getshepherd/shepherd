// a release build on Windows is a windowed app: no console window of its own
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ponytail: a release build on Windows has no console, so `hooks` prints nothing there
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "hooks") {
        std::process::exit(shepherd::herdr::hooks::cli(&args[1..]));
    }
    if args.first().is_some_and(|a| a == "device") {
        std::process::exit(shepherd::device::agent::cli(&args[1..]));
    }
    if args.first().is_some_and(|a| a == "browser") {
        std::process::exit(shepherd::browser_agent::cli(&args[1..]));
    }
    if args.first().is_some_and(|a| a == "skill") {
        std::process::exit(shepherd::agent_skill::cli(&args[1..]));
    }
    shepherd::logs::init();
    let rt = tokio::runtime::Runtime::new().expect("tokio");
    let handle = rt.handle().clone();
    let app = gpui_platform::application().with_assets(shepherd::assets::Assets);
    let (link_tx, mut links) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.on_open_urls(move |urls| urls.into_iter().for_each(|url| drop(link_tx.send(url))));
    app.run(move |cx| {
        gpui_base::init(cx);
        shepherd::notify::set_app_icon();
        shepherd::ui::command_panel::init(cx);
        shepherd::assets::register_fonts(cx);
        let path = shepherd::settings::Settings::path();
        // `SHEPHERD_WELCOME` shows the welcome card anyway (screenshots)
        let first_run = !path.exists() || std::env::var_os("SHEPHERD_WELCOME").is_some();
        let (settings, error) = shepherd::settings::Settings::load(&path);
        shepherd::herdr::set_binary(settings.herdr_bin.clone());
        shepherd::hc::set_sync(settings.hc_sync);
        shepherd::herdr::set_socket(settings.herdr_socket.clone());
        // a worktree hooks plugin and an agent skill an older Shepherd installed get this one's files
        std::thread::spawn(|| {
            if let Err(e) = shepherd::herdr::hooks::refresh() {
                log::warn!("worktree hooks: {e:#}");
            }
            if let Err(e) = shepherd::agent_skill::refresh() {
                log::warn!("agent skill: {e:#}");
            }
            shepherd::settings::prune_file_copies();
        });
        cx.set_global(settings.theme(cx.window_appearance()));
        shepherd::ui::follow_theme(cx);
        cx.observe_global::<shepherd::theme::Theme>(shepherd::ui::follow_theme).detach();
        shepherd::bindings::rebind(cx, &settings.keys);
        shepherd::bindings::follow_herdr_config(cx);
        cx.set_global(shepherd::settings::SettingsGlobal { settings, error, path });
        cx.on_action(|_: &shepherd::bindings::Quit, cx| cx.quit());
        cx.on_action(|_: &shepherd::bindings::RevealLogs, cx| cx.reveal_path(&shepherd::logs::path()));
        // off macOS an app with no windows left has nothing to show: closing the last one quits, as GNOME expects
        if !cfg!(target_os = "macos") {
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }
        shepherd::bindings::init_menu_actions(cx);
        cx.on_action(|_: &shepherd::bindings::OpenSettings, cx| shepherd::shell::Shell::toggle_settings(cx));
        use shepherd::{settings::step_font_size, ui::settings::update};
        cx.on_action(|_: &shepherd::bindings::FontSizeUp, cx| update(cx, |s| s.font_size = step_font_size(s.font_size, 1.0)));
        cx.on_action(|_: &shepherd::bindings::FontSizeDown, cx| update(cx, |s| s.font_size = step_font_size(s.font_size, -1.0)));
        cx.on_action(|_: &shepherd::bindings::ToggleSecureInput, cx| update(cx, |s| s.secure_input = !s.secure_input));
        cx.on_action(|_: &shepherd::bindings::FontSizeReset, cx| {
            update(cx, |s| s.font_size = shepherd::settings::Settings::default().font_size)
        });
        cx.set_global(shepherd::ui::settings::Tokio(handle.clone()));
        let (failures, mut failed) = tokio::sync::mpsc::unbounded_channel();
        shepherd::worktree::report_failures(failures);
        cx.spawn(async move |cx| {
            while let Some(message) = failed.recv().await {
                cx.update(|cx| shepherd::ui::toast::error(cx, message));
            }
        })
        .detach();
        shepherd::update::start(cx);
        shepherd::extensions::start(cx);
        shepherd::ui::loader::drive_slow_frames(cx);
        shepherd::analytics::seen([]);
        shepherd::notify::warn_if_denied(cx);

        shepherd::shell::Shell::open(handle.clone(), cx);
        if first_run {
            shepherd::shell::welcome(cx);
        }
        if let Some(why) = shepherd::logs::take_crash() {
            let reveal: std::rc::Rc<dyn Fn(&mut gpui::App)> = std::rc::Rc::new(|cx| cx.reveal_path(&shepherd::logs::path()));
            shepherd::ui::toast::info(cx, format!("Shepherd quit unexpectedly last time ({why}). Click to see the log."), Some(reveal));
        }
        if std::env::var_os("SHEPHERD_SETTINGS").is_some() {
            shepherd::shell::Shell::toggle_settings(cx);
        }
        // `SHEPHERD_PANEL=commands|agents|workspaces|sessions|browser[:url]|browser-menu|diffs|usage|projects|rename|review[:tab]|files[:path]|file:path`: opens that panel (`rename`, the
        // shown workspace's name being renamed in the sidebar) once the first snapshot is likely in (screenshots).
        // `devices`, or a device's key (`device:ios:<udid>`), opens the right panel's Device tab on it; `:tools` after it
        // opens its Tools panel, `:tools:overlay` the accessibility overlay too, `:text-size` the rail's Text size menu.
        // `extension:<name>` opens that extension's page in the right panel.
        if let Ok(which) = std::env::var("SHEPHERD_PANEL") {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                cx.update(|cx| {
                    let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<shepherd::shell::Shell>()) else { return };
                    let _ = window.update(cx, |shell, window, cx| shell.open_panel(&which, window, cx));
                })
            })
            .detach();
        }
        // `shepherd://` links (shell/links.rs), the first of them perhaps the one that launched the app: each waits
        // for the window and its first snapshot
        cx.spawn(async move |cx| {
            while let Some(url) = links.recv().await {
                for _ in 0..50 {
                    let followed = cx.update(|cx| {
                        let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<shepherd::shell::Shell>()) else { return false };
                        window.update(cx, |shell, window, cx| shell.follow_link(&url, window, cx)).unwrap_or(false)
                    });
                    if followed {
                        cx.update(|cx| cx.activate(true));
                        break;
                    }
                    cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
    drop(rt);
}
