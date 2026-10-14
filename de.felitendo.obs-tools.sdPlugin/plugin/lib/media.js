// Plays a file in an OBS media source, used by Video Player and Instant Replay.

const POLL_MS = 500;
const START_GRACE_MS = 1200;
const ENDED_STATES = new Set(["OBS_MEDIA_STATE_ENDED", "OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_NONE", "OBS_MEDIA_STATE_ERROR"]);

export function waitForEvent(client, type, { timeout = 10000, filter = () => true } = {}) {
	return new Promise((resolve, reject) => {
		const handler = (data) => {
			if (!filter(data)) return;
			cleanup();
			resolve(data);
		};
		const timer = setTimeout(() => {
			cleanup();
			reject(new Error(`OBS did not send ${type} in time`));
		}, timeout);
		const cleanup = () => {
			clearTimeout(timer);
			client.off(type, handler);
		};
		client.on(type, handler);
	});
}

async function findItem(client, sceneName, sourceName) {
	if (!sceneName) return null;
	try {
		const { sceneItemId } = await client.call("GetSceneItemId", { sceneName, sourceName });
		return sceneItemId;
	} catch {
		return null;
	}
}

/**
 * Starts playback and returns { done, stop }.
 * done resolves when the media ended (or was stopped) and the cleanup ran.
 */
export async function playMedia(client, state, options) {
	const { input, file, speed = 100, scene = "", showSource = true, hideWhenDone = true, returnToScene = false } = options;
	const previousScene = state.programScene;
	const targetScene = scene || state.programScene;

	// Load the file first, so the scene never shows the old one.
	const settings = { speed_percent: Math.min(200, Math.max(1, Math.round(Number(speed) || 100))), looping: false, restart_on_activate: true };
	if (file) Object.assign(settings, { is_local_file: true, local_file: file });
	await client.call("SetInputSettings", { inputName: input, inputSettings: settings, overlay: true });

	const itemId = showSource || hideWhenDone ? await findItem(client, targetScene, input) : null;
	if (showSource && itemId !== null) {
		await client.call("SetSceneItemEnabled", { sceneName: targetScene, sceneItemId: itemId, sceneItemEnabled: true });
	}
	await client.call("TriggerMediaInputAction", { inputName: input, mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" });
	if (scene && scene !== state.programScene) {
		await client.call("SetCurrentProgramScene", { sceneName: scene });
	}

	const startedAt = Date.now();
	let finished = false;
	let resolveDone;
	const done = new Promise((resolve) => (resolveDone = resolve));

	const finish = async () => {
		if (finished) return;
		finished = true;
		clearInterval(poll);
		client.off("MediaInputPlaybackEnded", onEnded);
		client.off("disconnected", finish);
		try {
			if (hideWhenDone && itemId !== null) {
				await client.call("SetSceneItemEnabled", { sceneName: targetScene, sceneItemId: itemId, sceneItemEnabled: false });
			}
			if (returnToScene && scene && previousScene && previousScene !== scene && state.programScene === scene) {
				await client.call("SetCurrentProgramScene", { sceneName: previousScene });
			}
		} catch {}
		resolveDone();
	};

	const onEnded = (data) => {
		if (data.inputName === input && Date.now() - startedAt > START_GRACE_MS / 2) finish();
	};
	client.on("MediaInputPlaybackEnded", onEnded);
	client.on("disconnected", finish);

	// The ended event is not sent in every case, so the status is checked too.
	const poll = setInterval(async () => {
		if (finished || Date.now() - startedAt < START_GRACE_MS) return;
		try {
			const status = await client.call("GetMediaInputStatus", { inputName: input });
			if (ENDED_STATES.has(status.mediaState)) finish();
		} catch {
			finish();
		}
	}, POLL_MS);

	const stop = async () => {
		if (finished) return;
		try {
			await client.call("TriggerMediaInputAction", { inputName: input, mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" });
		} catch {}
		await finish();
	};

	return { done, stop };
}
