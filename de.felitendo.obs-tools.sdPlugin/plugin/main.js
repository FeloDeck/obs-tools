// Entry point. OpenDeck starts this with node and the usual Stream Deck arguments.
import { ACTIONS } from "./actions/index.js";
import { setLanguage } from "./lib/i18n.js";
import { Plugin } from "./lib/plugin.js";
import { StreamDeck, parseArgs } from "./lib/streamdeck.js";

const [major] = process.versions.node.split(".").map(Number);
if (major < 22 || typeof WebSocket !== "function") {
	console.error(`OBS Tools needs Node.js 22 or newer (found ${process.version}).`);
	process.exit(1);
}

const args = parseArgs(process.argv.slice(2));
const sd = new StreamDeck(args);
const language = setLanguage(sd.info.application?.language);
const plugin = new Plugin(sd, ACTIONS);

sd.on("open", () => {
	sd.log(`OBS Tools started (Node ${process.version}, language ${language})`);
	sd.getGlobalSettings();
});
sd.on("close", () => {
	sd.log("OpenDeck closed the connection, exiting");
	process.exit(0);
});
sd.on("error", (error) => sd.log(`OpenDeck socket error: ${error?.message ?? error}`));

process.on("unhandledRejection", (error) => sd.log(`unhandled: ${error?.stack ?? error}`));
process.on("uncaughtException", (error) => sd.log(`uncaught: ${error?.stack ?? error}`));

sd.connect();
export { plugin };
