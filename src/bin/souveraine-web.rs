//! A wry host: self-hosted sites as apps, and the rig she wears.
//!
//! Two modes, one binary, because they are the same thing pointed at different
//! content — a webview on a Wayland surface with no browser around it.
//!
//! - `--url <URL>` wraps a site as an app. Own window, own `app_id`, so the
//!   compositor tiles it and the dock names it like anything else.
//! - `--rig <DIR>` is the avatar: transparent over the wallpaper, assets served
//!   from a custom scheme rather than `file://`, and an IPC line to the shell.
//!
//! wry binds the system webview (WebKitGTK here) rather than shipping a second
//! browser engine — which on a 3.5 GB daily driver is the whole argument.
//! Measured 2026-08-05: the Cubism runtime renders at ~58 fps in this engine on
//! blueline, 283 MB RSS.
//!
//! **This process holds no connection to the server.** `Souveraine.qml` is the
//! shell's one transport and stays that way; the face is a limb the shell
//! drives over `--ipc`, so the avatar and the sidebar are the same conversation
//! by construction rather than by two clients agreeing. Casey, 2026-08-05:
//! *"I will want it to be in sync with the sidebar — meaning if we 'resume'
//! it's resumed."*
//!
//! ## file:// is not enough
//!
//! `XMLHttpRequest` for the rig's `model.json` is blocked from `file://` even
//! with `allow-file-access-from-file-urls` set — it fails with status 0. The
//! phase-1 spike worked around it with a local HTTP server; shipping one to
//! serve our own assets would be a listening socket for no reason. A custom
//! scheme is wry's answer and costs nothing.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::http::Response;
use wry::WebViewBuilder;

/// What the shell can tell the face to do, and what it says back.
///
/// One JSON object per line, the same house grammar sessiond and viewtop speak,
/// so nothing here is a third idea of what talking to a process looks like.
#[derive(Debug)]
enum FromShell {
    /// Run a script in the page. Everything the shell drives — a line of her
    /// speech, a posture change, a motion — arrives as one of these, because
    /// the vocabulary belongs to the page's character layer and not to this
    /// file. See `docs/tasks/59-her-face-on-the-glass.md`.
    Eval(String),
    Quit,
    /// Where on this window touches actually land, in CSS pixels.
    ///
    /// A transparent window is still a rectangle to the compositor, and hers
    /// covers most of the home screen — so every tap meant for a widget
    /// underneath was being swallowed by empty glass she happens to occupy.
    /// `wl_surface.set_input_region` is the answer the protocol already has,
    /// and GTK reaches it through an input shape.
    ///
    /// The *policy* — a rectangle over her body now, her true silhouette
    /// later — belongs to the page, which is the only thing that knows where
    /// she is drawn. This carries whatever it decides.
    Shape(Vec<(i32, i32, i32, i32)>),
}

fn main() -> Result<()> {
    run()
}

fn run() -> Result<()> {
    let mut url: Option<String> = None;
    let mut rig: Option<PathBuf> = None;
    let mut ipc: Option<PathBuf> = None;
    let mut app_id = String::from("org.souveraine.web");
    let mut title = String::from("Souveraine");
    let mut transparent = false;
    let mut width = 540.0;
    let mut height = 960.0;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--rig" => rig = args.next().map(PathBuf::from),
            "--ipc" => ipc = args.next().map(PathBuf::from),
            "--app-id" => {
                if let Some(v) = args.next() {
                    app_id = v;
                }
            }
            "--title" => {
                if let Some(v) = args.next() {
                    title = v;
                }
            }
            "--transparent" => transparent = true,
            "--size" => {
                if let Some(v) = args.next() {
                    if let Some((w, h)) = v.split_once('x') {
                        width = w.parse().unwrap_or(width);
                        height = h.parse().unwrap_or(height);
                    }
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "souveraine-web --url <URL> [--app-id ID] [--size WxH] [--title T]\n\
                     souveraine-web --rig <DIR> --ipc <SOCK> --transparent [--title T]"
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }

    if url.is_none() && rig.is_none() {
        anyhow::bail!("one of --url or --rig is required");
    }

    // Before the event loop, because building it initialises GTK and GTK reads
    // the program name when it creates the surface — not when the window
    // handle is returned. Called after `build()` (where it used to be) the
    // compositor had already been told `souveraine-web`, so `--app-id` was a
    // flag that parsed, stored and did nothing. Measured 2026-08-06: the
    // compositor reported `souveraine-web`, which is why the only thing the
    // laptop's window rules could match on was the title.
    set_app_id(&app_id);

    let event_loop = EventLoopBuilder::<FromShell>::with_user_event().build();
    let shape_proxy = event_loop.create_proxy();

    // The X11 half, and it can only run here: `gdk::set_program_class` panics
    // if GDK is not initialised yet, and building the loop is what initialises
    // it. Wayland never reads this — `g_get_prgname()` above is what becomes
    // the app_id, and this is its fallback — so the ordering costs nothing.
    set_program_class(&app_id);

    let proxy = event_loop.create_proxy();

    // The shell drives this process; it never drives the shell. Reading on its
    // own thread and waking the loop through the proxy keeps the webview's
    // thread free, which matters because that thread is also the renderer.
    if let Some(path) = ipc.clone() {
        std::thread::spawn(move || serve_ipc(&path, proxy));
    }

    let window = WindowBuilder::new()
        .with_title(&title)
        .with_transparent(transparent)
        .with_decorations(!transparent)
        .with_inner_size(tao::dpi::LogicalSize::new(width, height))
        .build(&event_loop)
        .context("creating the window")?;

    let mut builder = WebViewBuilder::new()
        .with_transparent(transparent)
        // The character layer decides what a tap means and what she says; this
        // only carries it. Anything the page wants the shell to know goes out
        // the IPC as a line, so the shell stays the one thing talking to her.
        .with_ipc_handler(move |req| {
            let body = req.body();
            // The shape is ours to apply, not the shell's to route. It names
            // window geometry, which the shell has no opinion about and could
            // only hand straight back — and a round trip through the shell
            // would make the input region depend on the shell being up, which
            // is exactly when a face that eats every touch is worst.
            let shape = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .filter(|m| m.get("event").and_then(|v| v.as_str()) == Some("shape"));
            if let Some(msg) = shape {
                let rects: Vec<(i32, i32, i32, i32)> = msg
                    .get("rects")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| {
                                let r = r.as_array()?;
                                Some((
                                    r.first()?.as_f64()? as i32,
                                    r.get(1)?.as_f64()? as i32,
                                    r.get(2)?.as_f64()? as i32,
                                    r.get(3)?.as_f64()? as i32,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = shape_proxy.send_event(FromShell::Shape(rects));
                return;
            }
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{body}");
            let _ = out.flush();
        });

    if let Some(dir) = rig {
        let dir = dir.canonicalize().context("resolving the rig directory")?;
        // Loopback HTTP, not the custom scheme — and this is the second time
        // this exact wall has been hit. TASK-59 measured it for `file://`:
        // `XMLHttpRequest` for `model.json` fails with status 0 no matter what
        // access flags are set, because the origin is opaque. A custom scheme
        // is opaque in the same way, so the Cubism runtime — which fetches
        // `model.json`, the `.moc` and every texture over XHR — draws nothing
        // and reports no error, which is precisely the "canvas is there and
        // nothing draws" the task warned would cost an afternoon.
        //
        // The task preferred a custom scheme to avoid "a listening socket for
        // no reason". The reason turned out to be real. It binds 127.0.0.1 on
        // an ephemeral port, so it is reachable only from this machine and only
        // for as long as the face is up.
        let port = serve_rig_over_loopback(dir)?;
        builder = builder
            .with_initialization_script(INIT_SCRIPT)
            .with_url(format!("http://127.0.0.1:{port}/index.html"));
    } else if let Some(u) = url {
        builder = builder.with_url(u);
    }

    let webview = build_webview(builder, &window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::UserEvent(FromShell::Eval(script)) => {
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(FromShell::Shape(rects)) => {
                apply_input_shape(&window, &rects);
            }
            Event::UserEvent(FromShell::Quit)
            | Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

/// Read one JSON object per line and hand it to the loop.
fn serve_ipc(path: &std::path::Path, proxy: EventLoopProxy<FromShell>) {
    // The socket's parent directory is not guaranteed to exist: on the phone
    // sessiond or the compositor creates /run/user/NAME/souveraine/, but a
    // laptop with neither still needs the face reachable. Create it rather
    // than depend on whoever else got there first.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // A crashed host leaves its socket file behind, and a socket file that is
    // observed on blueline 2026-08-06: the face died, the shell's next join
    // died on "Address already in use", and the only fix was `rm`. Bind is the
    // moment to learn whether anything is actually listening: an error here
    // means the file is a corpse, so unlink it and take the bind once more.
    let listener = match std::os::unix::net::UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("[face] {} is stale; removing and retrying", path.display());
            let _ = std::fs::remove_file(path);
            match std::os::unix::net::UnixListener::bind(path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[face] cannot bind {}: {e}", path.display());
                    return;
                }
            }
        }
        Err(e) => {
            eprintln!("[face] cannot bind {}: {e}", path.display());
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                eprintln!("[face] unparseable: {line}");
                continue;
            };
            let event = match msg.get("op").and_then(|v| v.as_str()) {
                Some("eval") => msg
                    .get("script")
                    .and_then(|v| v.as_str())
                    .map(|s| FromShell::Eval(s.to_string())),
                Some("quit") => Some(FromShell::Quit),
                _ => None,
            };
            if let Some(event) = event {
                if proxy.send_event(event).is_err() {
                    return;
                }
            }
        }
    }
}

/// Serve the rig on 127.0.0.1, returning the port it landed on.
///
/// Deliberately the smallest thing that answers GET: the client is one webview
/// on the same machine fetching a dozen static files, so a dependency here
/// would be weight for nothing.
fn serve_rig_over_loopback(dir: PathBuf) -> Result<u16> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).context("binding the rig server")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let dir = dir.clone();
            // One thread per request. The webview opens a handful in parallel
            // for the textures, and a serial loop would deadlock the page
            // waiting on itself.
            std::thread::spawn(move || {
                let _ = answer_request(stream, &dir);
            });
        }
    });
    Ok(port)
}

fn answer_request(mut stream: std::net::TcpStream, dir: &std::path::Path) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    // Drain the headers so the client is not left writing into a full buffer.
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    let path = path.split('?').next().unwrap_or("/");
    let response = serve_asset(dir, path);
    let status = response.status().as_u16();
    let mime = response
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = response.body();
    write!(
        stream,
        "HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// Serve one file out of the rig directory.
///
/// Path traversal is refused rather than sanitised: the only correct answer to
/// `../../etc/passwd` is no.
fn serve_asset(root: &std::path::Path, path: &str) -> Response<std::borrow::Cow<'static, [u8]>> {
    let relative = path.trim_start_matches('/');
    let candidate = root.join(relative);
    let ok = candidate
        .canonicalize()
        .map(|p| p.starts_with(root))
        .unwrap_or(false);
    if !ok {
        return not_found();
    }
    let mime = match candidate.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("moc" | "mtn") => "application/octet-stream",
        _ => "application/octet-stream",
    };
    match std::fs::read(&candidate) {
        Ok(bytes) => Response::builder()
            .header("Content-Type", mime)
            .body(std::borrow::Cow::Owned(bytes))
            .unwrap_or_else(|_| not_found()),
        Err(_) => not_found(),
    }
}

fn not_found() -> Response<std::borrow::Cow<'static, [u8]>> {
    Response::builder()
        .status(404)
        .body(std::borrow::Cow::Borrowed(&[][..]))
        .expect("a 404 with an empty body is always well formed")
}

/// Injected before the page's own scripts.
///
/// Two jobs. The first is a one-line removal the upstream README lists as a
/// *feature*: `message.js` refuses to load on any user agent containing
/// "android", and this is a phone. The rig loads fine, the canvas is there, and
/// nothing draws — with no error. It is worth the injection rather than a patch
/// to the vendored file so that re-vendoring upstream cannot silently restore
/// it.
///
/// The second is the talk path. The reference opens an `EventSource` straight
/// at a chat API; here the page has no server to talk to, because this process
/// has no connection to one. It posts to the shell instead.
const INIT_SCRIPT: &str = r#"
(function () {
  // Look like a desktop to the character layer's mobile blocklist.
  try {
    Object.defineProperty(window.navigator, 'userAgent', {
      get: function () { return 'Mozilla/5.0 (X11; Linux x86_64) souveraine-web'; }
    });
  } catch (e) {}

  // Everything the page says, out on the same line protocol.
  //
  // Without this a rig that fails to draw is silent in every direction: the
  // canvas is there, WebGL reports fine, no exception reaches Rust, and the
  // only symptom is a transparent window. That cost this session an afternoon
  // of probing a live page over the IPC to ask it questions one at a time,
  // and DUMP-bugs-2026-08-06 §3 had already written down that the next step
  // when the avatar renders nothing is to capture the host's output.
  //
  // Runs in the initialisation script, so it is installed before any page
  // script — a message logged while the runtime is loading is exactly the one
  // worth having. The shell ignores what it does not recognise.
  try {
    ['log', 'warn', 'error'].forEach(function (level) {
      var original = console[level].bind(console);
      console[level] = function () {
        original.apply(null, arguments);
        try {
          window.ipc.postMessage(JSON.stringify({
            event: 'console',
            level: level,
            text: Array.prototype.map.call(arguments, String).join(' ')
          }));
        } catch (e) {}
      };
    });
    window.addEventListener('error', function (e) {
      try {
        window.ipc.postMessage(JSON.stringify({
          event: 'console',
          level: 'error',
          text: e.message + ' @ ' + (e.filename || '?') + ':' + (e.lineno || 0)
        }));
      } catch (_) {}
    });
    window.addEventListener('unhandledrejection', function (e) {
      try {
        window.ipc.postMessage(JSON.stringify({
          event: 'console', level: 'error', text: 'unhandled rejection: ' + e.reason
        }));
      } catch (_) {}
    });
  } catch (e) {}

  // The shell owns the conversation. Anything the page wants to say goes out
  // as a line; anything she says comes back as an eval that calls showMessage.
  window.souveraine = {
    say: function (text) {
      window.ipc.postMessage(JSON.stringify({ event: 'said', text: text }));
    },
    tapped: function (area) {
      window.ipc.postMessage(JSON.stringify({ event: 'tapped', area: area }));
    },
    // Speech edges only. The page now also has a small agent field when USB
    // Hands is joined, but the shell still owns the recorder, endpoint and
    // transcript; the webview remains a view over one conversation.
    talk: function (phase) {
      window.ipc.postMessage(JSON.stringify({ event: 'talk', phase: phase }));
    },
    hid: function (message) {
      var body = Object.assign({ event: 'hid' }, message || {});
      window.ipc.postMessage(JSON.stringify(body));
    },
    thread: function (mode) {
      window.ipc.postMessage(JSON.stringify({ event: 'thread', mode: mode }));
    },
    ready: function () {
      window.ipc.postMessage(JSON.stringify({ event: 'ready' }));
    },
    // The way back. A double tap dismisses her, which is the same gesture in
    // the same place that summoned her from the clock — and the clock cannot
    // be the way back, because by then it has faded out and a double tap on
    // an invisible object is not a route, it is a secret.
    dismiss: function () {
      window.ipc.postMessage(JSON.stringify({ event: 'dismiss' }));
    },
    // Where this window should take touches, in CSS pixels. Everything the
    // page does not name here falls through to whatever is behind her.
    shape: function (rects) {
      window.ipc.postMessage(JSON.stringify({ event: 'shape', rects: rects }));
    }
  };
  window.talkAPI = '';
})();
"#;

/// wry on Linux draws into the window's GTK container rather than adopting the
/// surface, so the build goes through the Unix extension. Kept in one place so
/// the two modes do not each grow a platform branch.
#[cfg(target_os = "linux")]
fn build_webview(
    builder: WebViewBuilder<'_>,
    window: &tao::window::Window,
) -> Result<wry::WebView> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;
    let vbox = window
        .default_vbox()
        .context("the window has no gtk container")?;
    builder.build_gtk(vbox).context("building the webview")
}

#[cfg(not(target_os = "linux"))]
fn build_webview(
    builder: WebViewBuilder<'_>,
    window: &tao::window::Window,
) -> Result<wry::WebView> {
    builder.build(window).context("building the webview")
}

/// Name the surface so the compositor can tile it and the dock can label it.
/// Without this every wrapped site is an untitled window, which is TASK-51's
/// "windows stack with nothing to tell them apart" arriving by a new route.
#[cfg(target_os = "linux")]
fn set_app_id(app_id: &str) {
    // On Wayland the app_id is GTK's program name, not a window property —
    // there is nothing on the handle to set, so this cannot wait for one to
    // exist. GTK reads the name while creating the surface, which happens
    // inside the window build, so the only place this works is before the
    // event loop is built at all. Pure glib, so it needs nothing initialised.
    gtk::glib::set_prgname(Some(app_id));
}

#[cfg(not(target_os = "linux"))]
fn set_app_id(_app_id: &str) {}

#[cfg(target_os = "linux")]
fn set_program_class(app_id: &str) {
    gtk::gdk::set_program_class(app_id);
}

/// Restrict where this window takes touches to the rectangles the page named.
///
/// The whole point of a transparent face is that the home screen is still
/// there — and it was not, because a surface with no input region takes every
/// contact inside its bounds whether it drew anything there or not. She is
/// 540px wide on a 540px panel, so that was the entire width of the screen.
///
/// An empty list is "all of it", not "none of it": a page that has not worked
/// out its shape yet, or one whose report was malformed, must not end up
/// untouchable. Losing taps to her is a nuisance; a face that cannot be
/// dismissed because nothing can reach it is a device you have to restart.
#[cfg(target_os = "linux")]
fn apply_input_shape(window: &tao::window::Window, rects: &[(i32, i32, i32, i32)]) {
    use gtk::prelude::WidgetExt;
    use tao::platform::unix::WindowExtUnix;

    let gtk_window = window.gtk_window();
    if rects.is_empty() {
        gtk_window.input_shape_combine_region(None);
        return;
    }
    let region = gtk::cairo::Region::create();
    for (x, y, w, h) in rects {
        if *w <= 0 || *h <= 0 {
            continue;
        }
        if region
            .union_rectangle(&gtk::cairo::RectangleInt::new(*x, *y, *w, *h))
            .is_err()
        {
            // A region that failed to build is not a region to install —
            // a partial one would silently make part of her untouchable.
            return;
        }
    }
    gtk_window.input_shape_combine_region(Some(&region));
}

#[cfg(not(target_os = "linux"))]
fn apply_input_shape(_window: &tao::window::Window, _rects: &[(i32, i32, i32, i32)]) {}

#[cfg(not(target_os = "linux"))]
fn set_program_class(_app_id: &str) {}
