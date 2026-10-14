import { Action } from "./base.js";
import { COLORS } from "../lib/icons.js";
import { num, t } from "../lib/i18n.js";
import { isColor, statKey } from "../lib/render.js";
import { formatDuration, timecodeSeconds } from "../lib/time.js";

const ID = "de.felitendo.obs-tools";

class StatsAction extends Action {
	onConnected() {
		this.unsubscribeStats?.();
		this.unsubscribeStats = this.obsState.subscribeStats();
		const onStats = (stats) => this.safe(() => this.onStats(stats));
		this.obsState.on("stats", onStats);
		const off = this.unsubscribeStats;
		this.unsubscribeStats = () => {
			off();
			this.conn?.state.off("stats", onStats);
		};
		if (this.obsState.stats) this.onStats(this.obsState.stats);
	}

	onDisconnected() {
		this.unsubscribeStats?.();
		this.unsubscribeStats = null;
	}

	onDisappear() {
		this.unsubscribeStats?.();
		this.unsubscribeStats = null;
	}

	onStats() {}
}

export class CpuUsage extends StatsAction {
	static uuid = `${ID}.cpuusage`;
	static defaults = { warnAt: 50, showMemory: false };

	onStats(stats) {
		this.stats = stats;
		this.render();
	}

	render() {
		if (!this.stats) return;
		const cpu = Number(this.stats.cpuUsage) || 0;
		const warn = Number(this.settings.warnAt) > 0 && cpu >= Number(this.settings.warnAt);
		this.setImage(
			statKey({
				label: t("OBS CPU"),
				value: `${num(cpu, cpu >= 100 ? 0 : 1)}%`,
				sub: this.settings.showMemory ? `${Math.round(this.stats.memoryUsage)} MB` : "",
				bar: Math.min(1, cpu / 100),
				color: warn ? COLORS.red : COLORS.blue,
				alert: warn ? COLORS.red : null,
			}),
		);
	}
}

const FRAME_TYPES = {
	dropped: { label: "DROPPED", read: (s) => (s.stream?.outputActive ? [s.stream.outputSkippedFrames, s.stream.outputTotalFrames] : null) },
	skipped: { label: "SKIPPED", read: (s) => [s.outputSkippedFrames, s.outputTotalFrames] },
	missed: { label: "MISSED", read: (s) => [s.renderSkippedFrames, s.renderTotalFrames] },
};

// Shows the frame count and flashes when it goes up. Press to stop the alarm.
export class DroppedFrames extends StatsAction {
	static uuid = `${ID}.droppedframes`;
	static defaults = { frameType: "dropped", threshold: 1, alertColor: COLORS.red, alertSeconds: 10 };

	onStats(stats) {
		const type = FRAME_TYPES[this.settings.frameType] ?? FRAME_TYPES.dropped;
		const values = type.read(stats);
		if (!values) {
			this.count = null;
			this.baseline = null;
			this.alertUntil = 0;
		} else {
			const [count, total] = values;
			if (this.baseline === null || this.baseline === undefined || count < this.baseline) this.baseline = count;
			if (count - this.baseline >= Math.max(1, Number(this.settings.threshold) || 1)) {
				this.alertUntil = Date.now() + Math.max(1, Number(this.settings.alertSeconds) || 10) * 1000;
				this.baseline = count;
			}
			this.count = count;
			this.total = total;
		}
		this.flash = !this.flash;
		this.render();
	}

	async onSettingsChanged(old) {
		if (old.frameType !== this.settings.frameType) {
			this.baseline = null;
			this.alertUntil = 0;
		}
	}

	render() {
		const type = FRAME_TYPES[this.settings.frameType] ?? FRAME_TYPES.dropped;
		const alerting = this.alertUntil > Date.now();
		const color = isColor(this.settings.alertColor) ? this.settings.alertColor : COLORS.red;
		if (this.count === null || this.count === undefined) {
			this.setImage(statKey({ label: t(type.label), value: "-", sub: this.settings.frameType === "dropped" ? t("not live") : "" }));
			return;
		}
		const pct = this.total > 0 ? (this.count / this.total) * 100 : 0;
		this.setImage(
			statKey({
				label: t(type.label),
				value: String(this.count),
				sub: `${num(pct, pct >= 10 ? 0 : 1)}%`,
				alert: alerting && this.flash ? color : null,
			}),
		);
	}

	// Press: stop the alarm and count from here.
	async onPress() {
		this.alertUntil = 0;
		this.baseline = this.count ?? null;
		this.render();
	}
}

function formatBitrate(kbps) {
	if (kbps === null || kbps === undefined || !Number.isFinite(kbps)) return "...";
	return kbps >= 1000 ? `${num(kbps / 1000, 1)} Mb/s` : `${Math.round(kbps)} kb/s`;
}

// Health of the stream at a glance: bitrate (or time), recent drops, and a green, yellow or red light.
export class StreamHealth extends StatsAction {
	static uuid = `${ID}.streamhealth`;
	static defaults = { show: "bitrate", warnDrops: 1, badDrops: 5 };

	onStats(stats) {
		const stream = stats.stream;
		this.samples ??= [];
		if (!stream?.outputActive) {
			this.samples = [];
			this.health = null;
		} else {
			// Drops of the last 10 seconds, so an old problem does not keep the key red.
			this.samples.push({ skipped: stream.outputSkippedFrames ?? 0, total: stream.outputTotalFrames ?? 0 });
			if (this.samples.length > 11) this.samples.shift();
			const first = this.samples[0];
			const last = this.samples.at(-1);
			const frames = last.total - first.total;
			const drops = frames > 0 ? ((last.skipped - first.skipped) / frames) * 100 : 0;
			const congestion = Number(stream.outputCongestion) || 0;
			const warn = Number(this.settings.warnDrops) || 1;
			const bad = Number(this.settings.badDrops) || 5;
			let level = "good";
			if (stream.outputReconnecting || drops >= bad || congestion >= 0.5) level = "bad";
			else if (drops >= warn || congestion >= 0.15) level = "warn";
			this.health = { level, drops: Math.max(0, drops), congestion, kbps: stream.kbps, timecode: stream.outputTimecode, reconnecting: stream.outputReconnecting };
		}
		this.render();
	}

	render() {
		const h = this.health;
		if (!h) {
			this.setImage(statKey({ label: t("STREAM"), value: t("off"), dot: COLORS.gray }));
			return;
		}
		const color = { good: COLORS.green, warn: COLORS.amber, bad: COLORS.red }[h.level];
		const value = h.reconnecting ? "..." : this.settings.show === "time" ? formatDuration(timecodeSeconds(h.timecode)) : formatBitrate(h.kbps);
		const sub = h.reconnecting ? t("reconnecting") : this.settings.show === "time" ? formatBitrate(h.kbps) : t("drops {pct}%", { pct: num(h.drops, h.drops >= 10 ? 0 : 1) });
		this.setImage(statKey({ label: t("LIVE"), value, sub, dot: color }));
	}
}
