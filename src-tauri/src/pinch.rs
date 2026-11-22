//! The touchpad pinch, handed to the page on Linux.
//!
//! On macOS a pinch reaches the page by itself: `WKWebView` never has
//! magnification switched on (wry does not touch `allowsMagnification`),
//! so WebKit forwards each `magnify` event as the DOM's own
//! `gesturestart`/`gesturechange`/`gestureend`, scale and all, and
//! `ui/src/lib/pinch.ts` listens for those. On Linux it never arrives.
//! WebKitGTK attaches its own `GtkGestureZoom` to the view and routes the
//! pinch into `ViewGestureController::setMagnification` — a Safari-style
//! magnification of the whole page, created unconditionally on process
//! launch — and the page sees one zero-delta "began" wheel event and nothing
//! else. It is not turned into Ctrl+wheel (Chromium's convention, which the
//! guard in `App.svelte` was written against), so nothing in JavaScript can
//! see it, let alone stop it (verified in `WebKitWebViewBase.cpp`,
//! 2026-09-03).
//!
//! So the pinch is taken off the widget before WebKit gets it. A
//! `GtkGestureZoom` of our own, on the same widget, in the **capture**
//! phase: GTK runs capture-phase controllers top-down before the target's
//! own handlers and before WebKit's bubble-phase gesture, and a sequence
//! claimed here is denied to every other gesture on the widget. What the
//! gesture reports — cumulative scale since the pinch began, and where the
//! fingers are, in widget coordinates, which are the page's CSS pixels — is
//! emitted to the page as a `pinch` event in the shape `pinch.ts` unifies
//! with the macOS one. The grid decides what a pinch means; this file only
//! delivers it.

/// Linux only, like the gesture that builds it: macOS gets the DOM's own
/// gesture events and never constructs one (a dead-code lint there otherwise).
#[cfg(target_os = "linux")]
#[derive(serde::Serialize, Clone)]
struct Pinch {
    /// `begin`, `update` or `end` — GTK's own phases, less `cancel`, which
    /// the page treats as an end (nothing to undo: the grid already stands
    /// at whatever the last update said).
    phase: &'static str,
    /// Relative to the height at `begin`, like the DOM's `GestureEvent.scale`.
    scale: f64,
    x: f64,
    y: f64,
}

/// Wires the capture-phase pinch gesture onto the main webview. Best effort:
/// no main window (headless `--sync-now` runs, tests) means nothing to wire,
/// and the app does not depend on it — the grid still zooms from
/// Ctrl+scroll and the keys.
#[cfg(target_os = "linux")]
pub fn install(app: &tauri::AppHandle) {
    use gtk::prelude::*;
    use tauri::{Emitter, Manager};

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    let _ = window.with_webview(move |webview| {
        let gesture = gtk::GestureZoom::new(&webview.inner());
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);

        let emit = move |gesture: &gtk::GestureZoom, phase: &'static str, scale: f64| {
            let (x, y) = gesture.bounding_box_center().unwrap_or((0.0, 0.0));
            if let Err(e) = handle.emit("pinch", Pinch { phase, scale, x, y }) {
                tracing::warn!(%e, "pinch not delivered to the page");
            }
        };

        let begin = emit.clone();
        gesture.connect_begin(move |g, _| {
            // Claimed at once, before WebKit's own zoom gesture — which runs
            // later, in the bubble phase — can take the sequence and magnify
            // the page under us.
            g.set_state(gtk::EventSequenceState::Claimed);
            begin(g, "begin", 1.0);
        });
        let update = emit.clone();
        gesture.connect_scale_changed(move |g, scale| update(g, "update", scale));
        let end = emit.clone();
        gesture.connect_end(move |g, _| end(g, "end", 1.0));
        gesture.connect_cancel(move |g, _| emit(g, "end", 1.0));

        // GTK 3 controllers are owned by whoever created them, not by the
        // widget (WebKit keeps its own alive with `g_object_set_data_full`).
        // Dropped here, the gesture would be destroyed the moment this
        // closure returns; it lives as long as the webview, which is the
        // life of the process.
        std::mem::forget(gesture);
    });
}

#[cfg(not(target_os = "linux"))]
pub fn install(_app: &tauri::AppHandle) {}
