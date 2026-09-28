#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Icon, WindowBuilder},
};
use timeline::bridge::Bridge;
use timeline::locale::{self, Language};
use wry::{DragDropEvent, WebContext, WebViewBuilder};

enum AppEvent {
    Message(Value),
    Ready(f64),
    Zoom(f64),
    Drop(String),
}

fn main() {
    if let Err(error) = run() {
        let language = Language::from_region(&locale::system_region());
        rfd::MessageDialog::new()
            .set_title(language.text("Timeline — ошибка запуска", "Timeline — startup error"))
            .set_description(format!(
                "{}\n\n{}",
                if language == Language::English {
                    locale::english_error(&error)
                } else {
                    format!("{error:#}")
                },
                language.text(
                    "Для запуска в Windows нужен Microsoft Edge WebView2 Runtime.",
                    "Microsoft Edge WebView2 Runtime is required on Windows."
                )
            ))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

fn run() -> anyhow::Result<()> {
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let icon = image::load_from_memory_with_format(
        include_bytes!("../ico/logo.ico"),
        image::ImageFormat::Ico,
    )?
    .into_rgba8();
    let icon = Icon::from_rgba(icon.as_raw().clone(), icon.width(), icon.height())?;
    let window = WindowBuilder::new()
        .with_title("Timeline")
        .with_window_icon(Some(icon))
        .with_inner_size(LogicalSize::new(1440., 900.))
        .with_min_inner_size(LogicalSize::new(860., 580.))
        .with_maximized(true)
        // Reveal only after CSS and the first layout are ready. No black/white flash.
        .with_visible(false)
        .build(&event_loop)?;
    let bridge = Arc::new(Bridge::default());
    let host = bridge.clone();
    let proxy = event_loop.create_proxy();
    let drop_proxy = proxy.clone();
    let html = include_str!("../web/index.html")
        .replace("/* INLINE_STYLE */", include_str!("../web/styles.css"))
        .replace("/* INLINE_I18N */", include_str!("../web/i18n.js"))
        .replace(
            "/* INLINE_VIEWER */",
            include_str!("../web/viewer.bundle.js"),
        )
        .replace("/* INLINE_SCRIPT */", include_str!("../web/app.js"));
    let profile = std::env::var_os("TIMELINE_PROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .or_else(|| std::env::var_os("XDG_DATA_HOME"))
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("Timeline")
                .join(if timeline::native::is_elevated() {
                    "WebView-admin"
                } else {
                    "WebView"
                })
        });
    std::fs::create_dir_all(&profile)?;
    let mut web_context = WebContext::new(Some(profile));
    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_initialization_script(
            std::env::args()
                .nth(2)
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .filter(Value::is_object)
                .map(|s| {
                    format!(
                        "localStorage.setItem('timeline.settings', {});",
                        serde_json::to_string(&s.to_string()).unwrap()
                    )
                })
                .unwrap_or_default(),
        )
        .with_initialization_script(format!(
            "window.timelineRegion = {};",
            serde_json::to_string(&locale::system_region())?
        ))
        .with_background_color((247, 248, 250, 255))
        .with_custom_protocol("timeline".into(), move |_, _| {
            wry::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(html.clone().into_bytes().into())
                .unwrap()
        })
        .with_url("timeline://localhost")
        .with_devtools(cfg!(debug_assertions))
        .with_hotkeys_zoom(false)
        .with_navigation_handler(|url| {
            matches!(
                url.as_str(),
                "timeline://localhost"
                    | "timeline://localhost/"
                    | "http://timeline.localhost/"
                    | "http://timeline.localhost"
            )
        })
        .with_ipc_handler(move |request| {
            let value = match serde_json::from_str::<Value>(request.body()) {
                Ok(value) => value,
                Err(_) => return,
            };
            if value["command"] == "ready" {
                let zoom = value["zoom"].as_f64().unwrap_or(1.0).clamp(0.5, 2.0);
                let _ = proxy.send_event(AppEvent::Ready(zoom));
                return;
            }
            if value["command"] == "zoom" {
                if let Some(factor) = value["factor"].as_f64().filter(|v| v.is_finite()) {
                    let _ = proxy.send_event(AppEvent::Zoom(factor.clamp(0.5, 2.0)));
                }
                return;
            }
            let response_proxy = proxy.clone();
            host.dispatch(value, move |message| {
                let _ = response_proxy.send_event(AppEvent::Message(message));
            });
        })
        .with_drag_drop_handler(move |event| {
            match event {
                DragDropEvent::Drop { paths, .. } => {
                    if let Some(path) = paths.first() {
                        let _ = drop_proxy
                            .send_event(AppEvent::Drop(path.to_string_lossy().into_owned()));
                    }
                }
                DragDropEvent::Enter { .. } => {
                    let _ = drop_proxy
                        .send_event(AppEvent::Message(json!({"event":"drag", "active":true})));
                }
                DragDropEvent::Leave => {
                    let _ = drop_proxy
                        .send_event(AppEvent::Message(json!({"event":"drag", "active":false})));
                }
                _ => {}
            }
            true
        });
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().unwrap())?
    };
    let mut argument = std::env::args_os()
        .nth(1)
        .map(|p| p.to_string_lossy().into_owned());
    let mut ready = false;
    let mut last_progress = Instant::now();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = if bridge.progress().is_some() {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100))
        } else {
            ControlFlow::Wait
        };
        let send = |value: Value| {
            let _ = webview.evaluate_script(&format!("window.timelineReceive({value})"));
        };
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                bridge.dispatch(json!({"id":0,"command":"cancel"}), |_| {});
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(AppEvent::Ready(zoom)) => {
                let _ = webview.zoom(zoom);
                if !ready {
                    ready = true;
                    window.set_visible(true);
                    // Showing an initially hidden window may restore its normal
                    // size on Windows; maximize after the first visible layout.
                    window.set_maximized(true);
                    if let Some(path) = argument.take() {
                        send(json!({"event":"open", "path":path}));
                    }
                }
            }
            Event::UserEvent(AppEvent::Zoom(factor)) => {
                let _ = webview.zoom(factor);
            }
            Event::UserEvent(AppEvent::Drop(path)) => send(json!({"event":"open", "path":path})),
            Event::UserEvent(AppEvent::Message(value)) => {
                if let Some(name) = value["data"]["name"].as_str() {
                    window.set_title(&format!("{name} — Timeline"));
                }
                send(value);
            }
            Event::MainEventsCleared
                if ready && last_progress.elapsed() >= Duration::from_millis(100) =>
            {
                last_progress = Instant::now();
                if let Some(progress) = bridge.progress() {
                    send(progress);
                }
            }
            _ => {}
        }
    });
}
