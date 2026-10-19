// What press, hold and double press can do on a Set Text key. The lists are in src/text_gestures.rs,
// `cargo xtask manifest` copies them into shared.js.
import { TEXT_ACTIONS, TEXT_GESTURES, TEXT_KINDS } from "./shared.js";

export { TEXT_ACTIONS, TEXT_GESTURES, TEXT_KINDS };

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
