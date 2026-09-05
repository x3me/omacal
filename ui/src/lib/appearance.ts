/** The appearance fields shared by startup, Settings' live preview, and IPC. */
export type EventCornerStyle = 'rounded' | 'square';

export type AppearancePreferences = {
  backgroundTransparency: number;
  eventTransparency: number;
  eventCornerStyle: EventCornerStyle;
};

/**
 * Applies absolute transparency: 0 is opaque and 100 is clear.
 *
 * Omacal opts out of Omarchy's whole-window opacity when these controls are
 * installed, then reproduces that former baseline in the stored defaults.
 * Keeping alpha on the two painted surfaces is what lets them move
 * independently without fading text, outlines, menus, or dialogs.
 */
export function applyAppearance(
  preferences: AppearancePreferences,
  root: HTMLElement = document.documentElement,
): void {
  const background = percent(preferences.backgroundTransparency);
  const events = percent(preferences.eventTransparency);

  setTransparency(root, 'backgroundTransparency', '--background-fill-opacity', background);
  setTransparency(root, 'eventTransparency', '--event-fill-opacity', events);

  if (preferences.eventCornerStyle === 'square') {
    root.dataset.eventCorners = 'square';
    root.style.setProperty('--event-card-radius', '0px');
    root.style.setProperty('--event-chip-radius', '0px');
    root.style.setProperty('--event-pill-radius', '0px');
  } else {
    delete root.dataset.eventCorners;
    root.style.removeProperty('--event-card-radius');
    root.style.removeProperty('--event-chip-radius');
    root.style.removeProperty('--event-pill-radius');
  }
}

function percent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function setTransparency(
  root: HTMLElement,
  dataKey: 'backgroundTransparency' | 'eventTransparency',
  property: string,
  transparency: number,
): void {
  root.dataset[dataKey] = String(transparency);
  root.style.setProperty(property, `${100 - transparency}%`);
}
