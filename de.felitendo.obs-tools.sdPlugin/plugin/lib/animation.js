// Source animations: phases of transform changes, played frame by frame inside OBS.
import { BatchExecution } from "./obs.js";

export const FIELDS = ["positionX", "positionY", "rotation", "scaleX", "scaleY", "cropLeft", "cropRight", "cropTop", "cropBottom", "boundsWidth", "boundsHeight"];
const CROP = new Set(["cropLeft", "cropRight", "cropTop", "cropBottom"]);
const BOUNDS = new Set(["boundsWidth", "boundsHeight"]);
const MAX_FRAMES = 60 * 120;

export const EASINGS = {
	linear: (t) => t,
	easeIn: (t) => t * t * t,
	easeOut: (t) => 1 - Math.pow(1 - t, 3),
	easeInOut: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
	back: (t) => 1 + 2.70158 * Math.pow(t - 1, 3) + 1.70158 * Math.pow(t - 1, 2),
};

function round(value, digits = 4) {
	const f = 10 ** digits;
	return Math.round(value * f) / f;
}

export function hasBounds(transform) {
	return !!transform && transform.boundsType && transform.boundsType !== "OBS_BOUNDS_NONE";
}

// What changed between two transforms, only the fields that moved.
export function diffTransforms(from, to) {
	const changes = {};
	for (const field of FIELDS) {
		if (BOUNDS.has(field) && !(hasBounds(from) && hasBounds(to))) continue;
		const a = Number(from?.[field]);
		const b = Number(to?.[field]);
		if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
		const delta = round(b - a);
		if (Math.abs(delta) > 1e-4) changes[field] = delta;
	}
	return changes;
}

export function normalizePhase(phase = {}) {
	const changes = {};
	for (const field of FIELDS) {
		const value = Number(phase.changes?.[field]);
		if (Number.isFinite(value) && value !== 0) changes[field] = value;
	}
	return {
		duration: Math.max(0, Number(phase.duration) || 0),
		delay: Math.max(0, Number(phase.delay) || 0),
		easing: EASINGS[phase.easing] ? phase.easing : "linear",
		changes,
	};
}

function value(field, number) {
	if (CROP.has(field)) return Math.max(0, Math.round(number));
	if (BOUNDS.has(field)) return Math.max(1, number);
	if ((field === "scaleX" || field === "scaleY") && Math.abs(number) < 0.0001) return number < 0 ? -0.0001 : 0.0001;
	return round(number, 5);
}

// Every transform to set, one per frame. Delays become empty frames.
export function buildFrames(start, phases, fps) {
	const normalized = phases.map(normalizePhase);
	const fields = FIELDS.filter((field) => normalized.some((p) => field in p.changes) && (!BOUNDS.has(field) || hasBounds(start)));
	const current = Object.fromEntries(fields.map((f) => [f, Number(start?.[f]) || 0]));
	const steps = [];
	for (const phase of normalized) {
		const wait = Math.round((phase.delay / 1000) * fps);
		if (wait > 0) steps.push({ wait });
		const frames = Math.max(1, Math.round((phase.duration / 1000) * fps));
		const ease = EASINGS[phase.easing];
		const from = { ...current };
		for (let i = 1; i <= frames; i++) {
			const t = ease(i / frames);
			const transform = {};
			for (const field of fields) transform[field] = value(field, from[field] + (phase.changes[field] ?? 0) * t);
			steps.push({ transform });
		}
		for (const field of fields) current[field] = from[field] + (phase.changes[field] ?? 0);
	}
	return { fields, steps: steps.slice(0, MAX_FRAMES) };
}

export function totalDuration(phases) {
	return phases.map(normalizePhase).reduce((sum, p) => sum + p.delay + p.duration, 0);
}

// The requests for one run, played by OBS on its own render frames.
export function buildRequests({ sceneName, sceneItemId, start, phases, fps, options = {}, sourceName }) {
	const { fields, steps } = buildFrames(start, phases, fps);
	const requests = [];
	const item = { sceneName, sceneItemId };
	const filter = (enabled) => ({ requestType: "SetSourceFilterEnabled", requestData: { sourceName, filterName: options.filter, filterEnabled: enabled } });
	if (options.filter && options.filterStart && options.filterStart !== "none") requests.push(filter(options.filterStart === "enable"));
	if (options.showAtStart) requests.push({ requestType: "SetSceneItemEnabled", requestData: { ...item, sceneItemEnabled: true } });
	for (const step of steps) {
		if (step.wait) {
			requests.push({ requestType: "Sleep", requestData: { sleepFrames: step.wait } });
		} else {
			requests.push({ requestType: "SetSceneItemTransform", requestData: { ...item, sceneItemTransform: step.transform } });
			requests.push({ requestType: "Sleep", requestData: { sleepFrames: 1 } });
		}
	}
	if (options.hideAtEnd) requests.push({ requestType: "SetSceneItemEnabled", requestData: { ...item, sceneItemEnabled: false } });
	if (options.resetAtEnd && fields.length) {
		if (options.hideAtEnd) requests.push({ requestType: "Sleep", requestData: { sleepFrames: 1 } });
		const reset = Object.fromEntries(fields.map((f) => [f, value(f, Number(start[f]) || 0)]));
		requests.push({ requestType: "SetSceneItemTransform", requestData: { ...item, sceneItemTransform: reset } });
	}
	if (options.filter && options.filterEnd && options.filterEnd !== "none") requests.push(filter(options.filterEnd === "enable"));
	return { requests, frames: steps.length };
}

export async function runAnimation(client, args) {
	const { requests, frames } = buildRequests(args);
	const timeout = (frames / Math.max(1, args.fps)) * 1000 + 15000;
	const results = await client.batch(requests, { executionType: BatchExecution.SERIAL_FRAME, haltOnFailure: true, timeout });
	const failed = results.find((r) => !r.requestStatus?.result);
	if (failed) throw new Error(failed.requestStatus?.comment || `${failed.requestType} failed`);
	return results.length;
}
