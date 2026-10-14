// What press, hold and double press can do on a Set Text key. All three offer the same.
// The settings page imports this file too, so keep it free of Node imports.
export const TEXT_ACTIONS = ["none", "set", "toggle", "next", "previous", "first", "up", "down", "reset", "timer", "clear"];
export const TEXT_GESTURES = ["press", "hold", "double"];

// What a text action shows on the key afterwards.
export const TEXT_KINDS = { set: "text", toggle: "text", next: "line", previous: "line", first: "line", up: "count", down: "count", reset: "count", timer: "timer", clear: "cleared" };

// The action of a gesture. Press sets the text unless something else is picked.
export function textGesture(settings, gesture) {
	const value = settings?.[gesture];
	if (TEXT_ACTIONS.includes(value)) return value;
	return gesture === "press" ? "set" : "none";
}

// Does any gesture use something of this kind ("text", "line", "count" or "timer")?
export function textUses(settings, kind) {
	return TEXT_GESTURES.some((gesture) => TEXT_KINDS[textGesture(settings, gesture)] === kind);
}
