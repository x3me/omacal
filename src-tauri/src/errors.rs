/// Error messages this app itself produces that are safe to show verbatim,
/// matched by exact prefix against the literal strings the code actually
/// emits.
///
/// An allowlist, not a deny-list: anything that does not match one of these
/// prefixes is withheld behind `OPAQUE`, including any error shape nobody has
/// thought of yet. A deny-list of secret markers can only ever cover the
/// secrets it was told to name — a 40-character bare token with no `Bearer`,
/// no `ya29.`, no `://`, would sail through one unrecognised. Defaulting to
/// withhold is the safer failure mode for a function whose only job is
/// secret-safety.
///
/// A message belongs here — rather than in [`SAFE_EXACT`] — only when it
/// genuinely carries variable, benign trailing detail of its own (a
/// filesystem path, a `std::io::Error`'s message). `starts_with` semantics
/// admit *anything* appended after the prefix, so a prefix entry for a
/// message with no legitimate variable part would also admit a future
/// `format!("{lit}: {detail}")` built by wrapping it in more context — the
/// exposure [`SAFE_EXACT`] exists to close.
///
/// Each entry below is cited against the one call site that emits it, checked
/// to confirm it interpolates nothing beyond what is already known-benign, and
/// that nothing further up the call chain wraps it in additional `.context(..)`
/// that could smuggle something else in ahead of it.
const SAFE_PREFIXES: &[&str] = &[
    // src-tauri/src/lib.rs:96 (`load_config`'s missing-file branch). Interpolates
    // the config path and a `std::io::Error` display (e.g. "No such file or
    // directory (os error 2)"); fires before any secret is ever read off disk,
    // so "client_secret" here can only ever be the literal key name.
    "no config at ",
    // src-tauri/src/events.rs — `split_series`' refusal to strand materialised
    // exceptions in the tail of a series it is about to split. A prefix rather
    // than an exact entry because it genuinely has a variable part: the number
    // of occupied slots, so the user knows the scale of what they are being
    // asked to redo.
    //
    // What follows the prefix is a `COUNT(*)` rendered by `format!` and nothing
    // else — an `i64` from `omacal_store::exceptions_from`, which cannot carry a
    // path, a URL or a token whatever the database holds. The `bail!` is
    // reached by a bare `?` with no `.context(..)` anywhere on its way to
    // `update_event`'s `.map_err(..)`, so nothing is appended after it either.
    // `a_split_that_would_strand_moved_occurrences_is_refused_before_any_write`
    // in `events.rs` pins the whole rendered string for a known count, so a
    // future wrap that started smuggling detail in after the number would fail
    // a test rather than pass unnoticed.
    "some later occurrences of this series were moved or deleted on their own, and a split \
     cannot carry them across — edit all events instead, or re-create them afterwards. \
     Occurrences affected: ",
];

/// Error messages matched as a whole, not a prefix: nothing may come before or
/// after one of these before `user_facing` will show it. Every entry here is a
/// fixed literal with no variable part of its own, so `starts_with` would be
/// strictly weaker than the prefix list already grants prefix-only strings
/// (see the doc comment above) — exact matching is what keeps a future
/// `format!("{lit}: {detail}")` that wraps one of these in more context from
/// silently starting to pass.
const SAFE_EXACT: &[&str] = &[
    // src-tauri/src/events.rs — `CREATED_NOT_STORED`, raised only after
    // Google's insert succeeded. Fixed literal, no interpolation. This one
    // MUST reach the user verbatim: swallowed into the opaque fallback it
    // reads as "the create failed", and the natural response to that mails
    // every guest a second invitation.
    crate::events::CREATED_NOT_STORED,
    // src-tauri/src/lib.rs — `BROWSER_FAILED`, raised where `sign_in` asks
    // the OS to open the consent page and the launcher fails or exits
    // non-zero. Fixed literal built by `anyhow!(BROWSER_FAILED)` with no
    // interpolation; the launcher's own detail goes to `tracing`, never
    // here. Withheld, this failure is indistinguishable from an OAuth
    // problem — which is issue #1's diagnosis story in one line.
    crate::BROWSER_FAILED,
    // crates/omacal-google/src/auth.rs:171 (the `TIMED_OUT` constant), raised
    // at auth.rs:206 with no interpolation and propagated to `sign_in_impl`
    // via a bare `?` with no `.context(..)` added along the way.
    "Sign-in timed out — no response from the browser. Try again.",
    // src-tauri/src/lib.rs:147 — the CSRF guard's abort. Fixed literal, no
    // interpolation; this is the check that must run before the code exchange.
    "state mismatch — possible CSRF, sign-in aborted",
    // src-tauri/src/lib.rs:169. Fixed literal, no interpolation.
    "account has no primary calendar",
    // src-tauri/src/lib.rs:174. Fixed literal, no interpolation.
    "Google returned no refresh token — revoke the app's access and retry",
    // src-tauri/src/events.rs — `event_detail_impl`'s missing-row branch (also
    // reached from `respond_impl`/`refresh_impl`'s own lookup), and
    // `respond_impl`'s two guard checks. All three are reached only through
    // `.map_err(|e| crate::errors::user_facing(&e))` with no `.context(..)`
    // added anywhere on the way, so `err.to_string()` is byte-identical to
    // each literal below.
    "that event is no longer here",
    "this calendar cannot be answered from OmaCal",
    "you are not a guest on this event",
    // src-tauri/src/events.rs — `create_impl`'s three guards: the missing-
    // calendar branch (a calendar removed between the picker and the save),
    // the demo gate, and the writability check. Reached only through
    // `create_event`'s `.map_err(|e| crate::errors::user_facing(&e))` with no
    // `.context(..)` added on the way, so each is byte-identical here too.
    "that calendar is no longer here",
    "demo mode — there is nothing to create",
    "this calendar is not writable from OmaCal",
    // src-tauri/src/events.rs — `update_impl`'s demo gate. A third fixed
    // literal for a third verb rather than one shared string: see
    // `DEMO_SYNC_MESSAGE`'s own doc comment in `lib.rs` for why each write
    // command says what *it* cannot do. Reached only through `update_event`'s
    // `.map_err(|e| crate::errors::user_facing(&e))` with no `.context(..)`
    // added on the way, so it is byte-identical here too. `update_impl`'s
    // other three refusals reuse literals already listed above.
    "demo mode — there is nothing to save",
    // src-tauri/src/events.rs — `delete_impl`'s demo gate. A fourth fixed
    // literal for a fourth verb, on the same reasoning as the third above.
    // Reached only through `delete_event_cmd`'s `.map_err(|e|
    // crate::errors::user_facing(&e))` with no `.context(..)` added on the way,
    // so it is byte-identical here too. `delete_impl`'s other three refusals
    // reuse literals already listed above.
    "demo mode — there is nothing to delete",
    // src-tauri/src/events.rs — `resolve_instance_id`'s empty-lookup branch on
    // a bare series master (reached from `respond_via_client`, called by
    // `respond_to_event`). Fixed literal, no interpolation, and reached via a
    // bare `?` with no `.context(..)` added anywhere between the `bail!` and
    // `.map_err(|e| crate::errors::user_facing(&e))`, so `err.to_string()` is
    // byte-identical to the literal below. Without this entry, the one RSVP
    // failure a user is likeliest to actually hit — clicking "This one" on an
    // occurrence the local store has no exception row for yet — read as
    // OPAQUE instead of naming what happened.
    "could not find that occurrence on the calendar",
    // src-tauri/src/events.rs — `row_from_wire`'s tombstone branch, reached
    // when the occurrence being edited was deleted between the popover opening
    // and the save. Fixed literal, no interpolation, raised with `bail!` and
    // propagated by a bare `?` with no `.context(..)` anywhere on the way to
    // `update_event`'s `.map_err(..)`. Deliberately narrower than the branch it
    // replaced: an event whose *times* will not parse is a shape nobody has
    // seen, and that one stays opaque rather than telling the user something
    // specific that may not be true.
    "that occurrence is no longer on the calendar",
    // src-tauri/src/events.rs — `update_via_client`'s refusal to retry a
    // **guest-list** change after a 412. Fixed literal, held in
    // `events::CONFLICT_GUESTS` and raised with `bail!(CONFLICT_GUESTS)`,
    // propagated by a bare `?` with no `.context(..)` anywhere on the way to
    // `update_event`'s `.map_err(..)`, so `err.to_string()` is byte-identical
    // to the constant. Entered here rather than left opaque because it is the
    // one conflict a user can actually act on — reopen the form and make the
    // change again — and because "something went wrong" for a guest list
    // nobody knows was dropped is exactly the silent failure guest-list spec
    // §2 is about. `a_guest_list_conflict_reaches_the_user_verbatim` pins the
    // pair, so a reworded constant fails a test rather than going opaque.
    crate::events::CONFLICT_GUESTS,
    // src-tauri/src/settings.rs — `set_sync_interval`'s floor. Fixed literal,
    // held in `settings::INTERVAL_TOO_SHORT` and raised with `bail!`, reached
    // only through `set_sync_interval`'s own `.map_err(user_facing)` with no
    // `.context(..)` on the way, so `err.to_string()` is byte-identical.
    // Allowlisted because the refusal is the whole point: a value accepted and
    // then quietly clamped is worse than one turned down, and a turned-down
    // value the user cannot read a reason for is barely better. Pinned by
    // `an_interval_below_the_floor_is_refused_and_nothing_is_stored`.
    crate::settings::INTERVAL_TOO_SHORT,
    // src-tauri/src/settings.rs — the default event duration cannot describe
    // a zero-length event. Fixed literal, raised before the settings write.
    crate::settings::EVENT_DURATION_TOO_SHORT,
    // src-tauri/src/events.rs — `split_series`' refusal to split a series that
    // ends after a fixed number of occurrences. Fixed literal, no
    // interpolation, raised with `bail!` before either write and propagated by
    // a bare `?` with no `.context(..)` on the way to `update_event`'s
    // `.map_err(..)`. Allowlisted because the user can act on it: "All events"
    // does what they wanted, and OPAQUE here would send them looking in a log
    // for a decision rather than a fault.
    "OmaCal cannot split a series that ends after a set number of times — \
     edit all events instead",
    // src-tauri/src/events.rs — `split_series`' second write failing after the
    // first landed. Built by `map_err` from a fixed literal that drops the
    // underlying `ApiError` entirely (it is logged instead), so no status line
    // or URL is interpolated into it, and propagated by a bare `?` with no
    // `.context(..)` added. **The one message in this list the user must act
    // on**: two overlapping series are on their calendar, both render, and
    // nothing else in the app will ever tell them. OPAQUE here would send them
    // looking for an edit that did not happen instead of a duplicate that did.
    "the new series was created but the original could not be shortened — \
     you now have two overlapping series and should delete one",
    // crates/omacal-caldav/src/client.rs — `https_or_private`'s refusal of a
    // plain-http address outside the user's own network, reached from
    // `CalDavClient::new` through `connect_caldav`'s `.map_err(user_facing)`
    // with no `.context(..)` on the way. The constant interpolates nothing on
    // purpose (the rejected URL can carry a password in its userinfo), and
    // `the_scheme_refusal_does_not_repeat_the_address_back` pins that.
    //
    // Allow-listed because OPAQUE here is the whole of issue #28: a
    // self-hosted CalDAV server that works in every other client is turned
    // down for a knowable, fixable reason, and "Sync failed, see the log"
    // sends the user looking for a fault instead of reading a decision.
    omacal_caldav::NOT_PRIVATE_HTTP,
    // src-tauri/src/events.rs — `move_target`'s two refusals, both raised with
    // `bail!` on a fixed literal before any write happens, and reaching the
    // user through `update_event`'s own `.map_err(user_facing)` with no
    // `.context(..)` on the way.
    //
    // Allowlisted because each names the action that resolves it: one says to
    // choose All events (a control already on screen), the other says the
    // destination is on another account (which is why the picker offered it
    // greyed). OPAQUE for either would turn a decision the user can act on
    // into a fault report about a save that did nothing.
    crate::events::MOVE_ONE_OCCURRENCE,
    crate::events::MOVE_ACROSS_ACCOUNTS,
    // src-tauri/src/events.rs — `MOVED_NOT_REMOVED`, raised by
    // `caldav_write::move_to` when the copy landed and the original would not
    // go. Fixed literal; the transport error that caused it is logged and
    // never interpolated. **Must** reach the user verbatim for
    // `CREATED_NOT_STORED`'s reason in the other direction: the event is on
    // two calendars, only the user can say which copy to delete, and nothing
    // else in the app will ever tell them.
    crate::events::MOVED_NOT_REMOVED,
];

/// The generic replacement. Deliberately says where to look rather than
/// pretending nothing happened.
const OPAQUE: &str = "Sync failed. See the application log for details.";

/// Renders an error for display in the webview.
///
/// Errors reach the UI through two channels — the `sync-failed` event and a
/// command's `Err` return — and both end up in the same header element. The
/// event channel already refuses to carry error detail; this is the other one.
pub fn user_facing(err: &anyhow::Error) -> String {
    let text = err.to_string();

    if SAFE_EXACT.iter().any(|s| text == *s) {
        return text;
    }
    if SAFE_PREFIXES.iter().any(|p| text.starts_with(p)) {
        return text;
    }

    OPAQUE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toml_parse_error_never_reaches_the_user_verbatim() {
        // toml's Display quotes the offending source line, which for this file
        // is the client secret. Plan 1b established this for the sync-failed
        // event; the command return path had the same hole.
        let src = "client_id = \"x\"\nclient_secret = GOCSPX-pretend-secret\n";
        let err: anyhow::Error = toml::from_str::<toml::Value>(src).unwrap_err().into();
        let shown = user_facing(&err);
        assert!(!shown.contains("GOCSPX"), "secret leaked to the UI: {shown}");
    }

    #[test]
    fn a_url_bearing_error_is_not_shown_verbatim() {
        // reqwest's Display carries the whole request URL, sync tokens included.
        let err = anyhow::anyhow!(
            "error sending request for url (https://x/events?syncToken=CPjO_SECRET)"
        );
        let shown = user_facing(&err);
        assert!(!shown.contains("syncToken"), "sync token leaked: {shown}");
        assert!(!shown.contains("CPjO_SECRET"));
    }

    #[test]
    fn a_safe_message_is_passed_through_so_the_user_can_act() {
        // The missing-config message names the file to create — losing it would
        // make the most common first-run failure unactionable.
        let err = anyhow::anyhow!(
            "no config at /Users/x/.config/omacal/config.toml: No such file or directory (os error 2). Create it with client_id and client_secret."
        );
        let shown = user_facing(&err);
        assert!(shown.contains("config.toml"));
        assert!(shown.contains("client_id"));
    }

    /// The created-not-stored sentence must cross to the UI verbatim: eaten
    /// by the opaque fallback it reads as "the create failed", and the
    /// natural answer to that mails every guest a second invitation. Asserted
    /// through `user_facing` itself, so removing the safelist entry — not
    /// only the constant — goes red.
    #[test]
    fn the_created_not_stored_sentence_reaches_the_user_whole() {
        let err = anyhow::anyhow!(crate::events::CREATED_NOT_STORED);
        assert_eq!(user_facing(&err), crate::events::CREATED_NOT_STORED);
    }

    #[test]
    fn an_unrecognised_error_falls_back_to_something_generic() {
        let err = anyhow::anyhow!("Bearer ya29.a0AfB_pretend_access_token failed");
        let shown = user_facing(&err);
        assert!(!shown.contains("ya29"), "access token leaked: {shown}");
        assert!(!shown.is_empty());
    }

    /// The match is a *prefix* match, and only a prefix match — for both lists,
    /// since `SAFE_EXACT` is (deliberately) a strictly narrower check than
    /// `starts_with` would be.
    ///
    /// Every other test here uses a string containing no safe string anywhere,
    /// so none of them can tell `starts_with`/`==` from `contains` — swapping
    /// either for `contains` left the whole suite green. That distinction is
    /// the entire safety property: what leads a message is text this app
    /// wrote, whereas what appears further in can be anything an error we
    /// wrapped chose to say. An error is safe because of who wrote its opening
    /// words, not because those words appear somewhere in it.
    #[test]
    fn a_safe_string_appearing_mid_message_is_still_withheld() {
        for safe in SAFE_PREFIXES.iter().chain(SAFE_EXACT.iter()) {
            let err = anyhow::anyhow!(
                "the token endpoint rejected the request: {safe}: ya29.a0AfB_pretend_token"
            );
            let shown = user_facing(&err);
            assert_eq!(
                shown, OPAQUE,
                "a safe string buried mid-message got the whole message shown: {shown}"
            );
            assert!(!shown.contains("ya29"), "token leaked behind a mid-string safe string: {shown}");
        }
    }

    /// The pair to the test above: the same prefixes, leading, must still pass —
    /// or the allowlist could be made vacuously safe by matching nothing.
    #[test]
    fn every_safe_prefix_still_passes_when_it_leads() {
        for safe in SAFE_PREFIXES {
            let err = anyhow::anyhow!("{safe}");
            assert_eq!(user_facing(&err), *safe, "a safe prefix was withheld");
        }
    }

    /// `every_exact_message_passes_through_unchanged` below only ever proves
    /// that whatever is *currently* in `SAFE_EXACT` behaves correctly — it
    /// cannot catch this specific literal being missing from the list in the
    /// first place. This is the RSVP failure a user is likeliest to actually
    /// hit (`resolve_instance_id`, reached by clicking "This one" on an
    /// occurrence the local store has no exception row for yet), so it gets
    /// its own direct check rather than relying only on the loop below.
    #[test]
    fn the_missing_occurrence_error_reaches_the_user_unobscured() {
        let err = anyhow::anyhow!("could not find that occurrence on the calendar");
        assert_eq!(user_facing(&err), "could not find that occurrence on the calendar");
    }

    /// The list's *membership*, named as data rather than read back off the
    /// list itself. Every other test here proves only that whatever
    /// `SAFE_EXACT` currently holds behaves correctly — delete an entry and
    /// they all stay green while the message it named silently starts reading
    /// as `OPAQUE`.
    ///
    /// This guards the UX direction, not the leak direction. A *missing*
    /// entry makes `user_facing` strictly more conservative: nothing escapes,
    /// the user just loses an actionable message and is told to read the log
    /// instead. The leak direction needs a human to wrongly *add* an entry
    /// for a message that interpolates something, which is what the rule in
    /// `SAFE_EXACT`'s own doc comment is for and what no test can check —
    /// hence the length assertion, so an addition has to pass back through
    /// that rule here rather than arriving unremarked.
    #[test]
    fn every_message_the_app_relies_on_showing_is_still_allowlisted() {
        const EXPECTED: &[&str] = &[
            "Sign-in timed out — no response from the browser. Try again.",
            "state mismatch — possible CSRF, sign-in aborted",
            "account has no primary calendar",
            "Google returned no refresh token — revoke the app's access and retry",
            "that event is no longer here",
            "this calendar cannot be answered from OmaCal",
            "you are not a guest on this event",
            "could not find that occurrence on the calendar",
            "that calendar is no longer here",
            "demo mode — there is nothing to create",
            "this calendar is not writable from OmaCal",
            "demo mode — there is nothing to save",
            "demo mode — there is nothing to delete",
            "that occurrence is no longer on the calendar",
            "OmaCal cannot split a series that ends after a set number of times — \
             edit all events instead",
            "the new series was created but the original could not be shortened — \
             you now have two overlapping series and should delete one",
            crate::events::CONFLICT_GUESTS,
            crate::settings::INTERVAL_TOO_SHORT,
            // Fixed literal raised before the settings write, with no
            // interpolation or context added on the command path.
            crate::settings::EVENT_DURATION_TOO_SHORT,
            // Checked against the doc-comment rule: a fixed literal raised
            // only after Google's insert succeeded, no interpolation, and
            // `create_event`'s `.map_err(user_facing)` adds no context.
            crate::events::CREATED_NOT_STORED,
            // Fixed literal raised by `anyhow!(BROWSER_FAILED)` where sign-in
            // asks the OS for a browser; the launcher's own error goes to
            // `tracing`, never into this string, and no `.context(..)` wraps
            // it on the way to `sign_in_impl`'s `map_err`.
            crate::BROWSER_FAILED,
            // Checked against the doc-comment rule: a fixed literal raised by
            // `bail!(NOT_PRIVATE_HTTP)` in the caldav crate's transport guard,
            // interpolating nothing (the address it refused is deliberately
            // not repeated back — it can carry a password in its userinfo),
            // and `connect_caldav`'s `.map_err(user_facing)` adds no context.
            omacal_caldav::NOT_PRIVATE_HTTP,
            // Checked against the doc-comment rule: three fixed literals, no
            // interpolation, each raised with `bail!` and propagated by a bare
            // `?` to `update_event_body`'s `.map_err(user_facing)`.
            crate::events::MOVE_ONE_OCCURRENCE,
            crate::events::MOVE_ACROSS_ACCOUNTS,
            crate::events::MOVED_NOT_REMOVED,
        ];
        for expected in EXPECTED {
            assert!(
                SAFE_EXACT.contains(expected),
                "`{expected}` is no longer allowlisted: the user now reads \"{OPAQUE}\" instead \
                 of a message that told them what happened"
            );
        }
        assert_eq!(
            SAFE_EXACT.len(),
            EXPECTED.len(),
            "SAFE_EXACT gained an entry this test does not name — add it above, having first \
             checked it against the rule in SAFE_EXACT's doc comment: a fixed literal, no \
             interpolation, no `.context(..)` anywhere on its way here"
        );
    }

    /// A guest-list conflict is the one write failure a user can actually act
    /// on — reopen the form and make the change again — so it must reach them
    /// as itself rather than as `OPAQUE`.
    ///
    /// Pinned as the pair it is: the literal `update_via_client` raises, and
    /// the entry that lets it through. Either one reworded on its own turns a
    /// specific, actionable message into "something went wrong" for a change
    /// the user has no other way of learning was dropped, which is the silent
    /// failure guest-list spec §2 is about.
    #[test]
    fn a_guest_list_conflict_reaches_the_user_verbatim() {
        let raised = anyhow::anyhow!(crate::events::CONFLICT_GUESTS);
        assert_eq!(user_facing(&raised), crate::events::CONFLICT_GUESTS);
        assert_ne!(user_facing(&raised), OPAQUE);
    }

    /// Issue #28: the address was refused for a reason the user can act on,
    /// and OPAQUE turns that decision into a fault report.
    #[test]
    fn a_refused_caldav_address_tells_the_user_why() {
        // `.err().expect(..)`: `CalDavClient` holds a password and has no `Debug`.
        let raised = omacal_caldav::CalDavClient::new("http://cal.example.com/", "u", "p")
            .err()
            .expect("a public http address must be refused");
        assert_eq!(user_facing(&raised), omacal_caldav::NOT_PRIVATE_HTTP);
        assert_ne!(user_facing(&raised), OPAQUE);
    }

    /// [`every_message_the_app_relies_on_showing_is_still_allowlisted`]'s rule,
    /// for the list that has no business being the one without it.
    ///
    /// `SAFE_PREFIXES` is the *weaker* of the two checks: `starts_with`
    /// semantics admit anything at all after the prefix, so an entry here is a
    /// standing promise that whatever the code appends to that literal is
    /// benign — a promise no test can verify, which is exactly why an addition
    /// has to pass back through the rule in `SAFE_PREFIXES`' doc comment rather
    /// than arriving unremarked. Both directions of the guard mirror the one
    /// above: the membership loop catches a deletion (the user silently loses
    /// an actionable message), the length assertion catches an addition.
    ///
    /// Written out as data rather than read off the list, for the same reason
    /// as the exact list's: every other test in this module iterates
    /// `SAFE_PREFIXES` itself and so agrees with it by construction, whatever
    /// it happens to contain.
    #[test]
    fn every_prefix_the_app_relies_on_showing_is_still_allowlisted() {
        const EXPECTED: &[&str] = &[
            "no config at ",
            "some later occurrences of this series were moved or deleted on their own, and a \
             split cannot carry them across — edit all events instead, or re-create them \
             afterwards. Occurrences affected: ",
        ];
        for expected in EXPECTED {
            assert!(
                SAFE_PREFIXES.contains(expected),
                "`{expected}` is no longer allowlisted: the user now reads \"{OPAQUE}\" instead \
                 of a message that told them what happened"
            );
        }
        assert_eq!(
            SAFE_PREFIXES.len(),
            EXPECTED.len(),
            "SAFE_PREFIXES gained an entry this test does not name — add it above, having first \
             checked it against the rule in SAFE_PREFIXES' doc comment: this list admits \
             *anything* after the prefix, so the variable part must be known-benign at every \
             call site that emits it, and nothing on the way to `user_facing` may wrap it in \
             further `.context(..)`"
        );
    }

    /// Same pairing for the exact-match list.
    #[test]
    fn every_exact_message_passes_through_unchanged() {
        for safe in SAFE_EXACT {
            let err = anyhow::anyhow!("{safe}");
            assert_eq!(user_facing(&err), *safe, "a safe exact message was withheld");
        }
    }

    /// The property exact matching exists for: unlike a prefix, nothing may
    /// follow one of these strings either. A future
    /// `format!("{lit}: {api_error}")` built by wrapping one of these literals
    /// in more context must not silently start passing through just because it
    /// begins with already-allowlisted text.
    #[test]
    fn an_exact_message_with_something_appended_is_withheld() {
        for safe in SAFE_EXACT {
            let err = anyhow::anyhow!("{safe}: unexpected detail that must not reach the ui");
            let shown = user_facing(&err);
            assert_eq!(
                shown, OPAQUE,
                "an exact-match string admitted trailing text: {shown}"
            );
        }
    }

    #[test]
    fn an_unrecognised_error_is_withheld_even_without_a_known_marker() {
        // A bare 40-char token with no scheme, no `Bearer`, no `ya29.` — none of
        // the shapes a deny-list would have thought to name. The allowlist
        // withholds it not because it recognises the token, but because it
        // recognises nothing here at all: default-to-withhold, not
        // default-to-pass.
        let err = anyhow::anyhow!("failed: a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
        let shown = user_facing(&err);
        assert!(
            !shown.contains("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"),
            "unrecognised secret leaked: {shown}"
        );
    }
}
