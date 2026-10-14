// The key images from the manifest (images/keys/<name>.svg). scripts/build-icons.mjs writes them
// from this table, and keys draw them again with a title below (see Action.stateImage).
// Gray means "off". The glyph is always in the middle: OpenDeck shows these pictures before the
// plugin draws a key, and then there is no title yet.
import { COLORS } from "./icons.js";

const G = COLORS.gray;

export const KEY_IMAGES = {
	scene: { glyph: "scene" },
	animation: { glyph: "animation" },
	"animation-on": { glyph: "animation", color: COLORS.green },
	studio: { glyph: "studio", color: G },
	"studio-on": { glyph: "studio" },
	video: { glyph: "video" },
	image: { glyph: "image" },
	"image-on": { glyph: "image", color: COLORS.green },
	"video-on": { glyph: "videoStop", color: COLORS.green },
	volume: { glyph: "volumeUp" },
	unmuted: { glyph: "unmuted" },
	muted: { glyph: "mute", color: COLORS.red },
	monitor: { glyph: "monitor", color: G },
	"monitor-on": { glyph: "monitor" },
	dropped: { glyph: "dropped" },
	cpu: { glyph: "cpu" },
	back: { glyph: "back" },
	// Profiles save the picture path. Last Scene keys made before the rename still point here.
	previous: { glyph: "back" },
	next: { glyph: "next" },
	replay: { glyph: "replay", color: G },
	"replay-on": { glyph: "replay" },
	browser: { glyph: "browser" },
	record: { glyph: "record" },
	"record-on": { glyph: "record", color: COLORS.red },
	stream: { glyph: "stream" },
	"stream-on": { glyph: "stream", color: COLORS.red },
	transition: { glyph: "transition", color: G },
	"transition-on": { glyph: "transition" },
	filter: { glyph: "filter", color: G },
	"filter-on": { glyph: "filter" },
	virtualcam: { glyph: "virtualcam", color: G },
	"virtualcam-on": { glyph: "virtualcam" },
	window: { glyph: "window" },
	visibility: { glyph: "eyeOff", color: G },
	"visibility-on": { glyph: "eye" },
	hotkey: { glyph: "keyboard" },
	screenshot: { glyph: "camera" },
	chapter: { glyph: "chapter", color: G },
	media: { glyph: "playPause" },
	profile: { glyph: "profile", color: G },
	text: { glyph: "text" },
	health: { glyph: "health" },
};
