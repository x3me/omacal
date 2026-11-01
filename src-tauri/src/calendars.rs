use crate::AppState;
use omacal_store::CalendarRow;

#[tauri::command]
pub async fn get_calendars(state: tauri::State<'_, AppState>) -> Result<Vec<CalendarRow>, String> {
    omacal_store::list_calendars(&state.pool)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

#[tauri::command]
pub async fn set_calendar_selected(
    state: tauri::State<'_, AppState>,
    id: i64,
    on: bool,
) -> Result<(), String> {
    omacal_store::set_selected(&state.pool, id, on)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// Shows exactly these calendars and hides the rest of the sync-enabled ones
/// (the ones the picker can toggle): applying a calendar group.
/// All-or-nothing, and display only — see
/// `omacal_store::set_selected_exactly`. The UI refreshes the way it does
/// after a single toggle.
#[tauri::command]
pub async fn set_calendars_shown(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<(), String> {
    omacal_store::set_selected_exactly(&state.pool, &ids)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// Sets or clears a calendar's colour, **locally and only locally**.
///
/// `hex` of `None` clears the override and the calendar follows Google's own
/// colour again — a different state from setting it to whatever Google happens
/// to use today, which would silently stop following it.
///
/// **Nothing here reaches Google.** This is a display preference of this
/// install's: the user's phone, the web UI, and anyone else subscribed to the
/// same calendar are untouched. That is the whole reason it is a column beside
/// Google's colour rather than a `calendarList.patch`.
#[tauri::command]
pub async fn set_calendar_color(
    state: tauri::State<'_, AppState>,
    id: i64,
    hex: Option<String>,
) -> Result<(), String> {
    omacal_store::set_color_override(&state.pool, id, hex.as_deref())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// Returns how many events were removed, so the UI can say what happened.
#[tauri::command]
pub async fn set_calendar_sync(
    state: tauri::State<'_, AppState>,
    id: i64,
    on: bool,
) -> Result<u64, String> {
    omacal_store::set_sync_enabled(&state.pool, id, on)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

#[tauri::command]
pub async fn set_calendar_label(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    label: Option<String>,
) -> Result<(), String> {
    omacal_store::set_label_override(&state.pool, id, label.as_deref())
        .await.map_err(|e| crate::errors::user_facing(&e))?;
    crate::settings::refresh_menu_surfaces(&app, &state).await;
    Ok(())
}
