//! CalDAV for omacal: iCloud and any standards-speaking server.
//!
//! Two halves. `client` is the wire protocol — discovery, listing, windowed
//! queries, etag-guarded writes — with the credential-safety rules built in
//! (HTTPS outside the user's own network, same-site hrefs, same-host
//! redirects). `ics` is the payload
//! format, kept deliberately small because recurrence lines pass through
//! verbatim to `omacal-core`'s expander.
//!
//! iCloud is not special-cased anywhere in this crate: it is a CalDAV server
//! at `https://caldav.icloud.com` that authenticates with an app-specific
//! password and answers discovery with partition-host hrefs, all of which the
//! generic paths handle. Keeping it unspecial is what makes Nextcloud,
//! Fastmail, Radicale and friends work for free.

pub mod client;
pub mod ics;

pub use client::{CalDavClient, CalDavError, DiscoveredCalendar, Resource, NOT_PRIVATE_HTTP};
pub use ics::{
    escape, events_in, exclude_occurrence, new_event_ics, new_todo_ics, parse, parse_time,
    patch_todo_fields, patch_todo_status, resolve, respond_all, respond_occurrence,
    rewrite_master, todos_in, truncate_series, unescape, upsert_exception, AttendeeWrite,
    CalAttendee,
    CalEvent, CalTodo, Component, EventWrite, IcsTime, Property, TodoDue, TodoEdit, WriteTime,
};

/// The fixed discovery address that makes "iCloud" a one-field sign-in.
pub const ICLOUD_BASE: &str = "https://caldav.icloud.com/";
