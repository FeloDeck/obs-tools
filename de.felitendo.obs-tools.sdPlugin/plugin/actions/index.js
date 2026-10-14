import { InputVolume, MonitorSet, MuteToggle, SourceVolume } from "./audio.js";
import { Hotkey, MediaControl, ProfileSwitch, RecordTools, Screenshot, SetText, SourceVisibility } from "./basics.js";
import { FilterDial, SceneDial, TransitionDial } from "./dials.js";
import { InstantReplay, RecordToggle, StreamToggle, VirtualCam } from "./outputs.js";
import { NextScene, PreviousScene, SetTransition, SmartScene, StudioMode } from "./scenes.js";
import { BrowserSource, FilterToggle, FocusedWindow, ImageSource, SourceAnimation, VideoPlayer } from "./sources.js";
import { CpuUsage, DroppedFrames, StreamHealth } from "./stats.js";

export const ACTIONS = [
	SmartScene,
	SourceAnimation,
	StudioMode,
	VideoPlayer,
	ImageSource,
	SourceVolume,
	InputVolume,
	MuteToggle,
	MonitorSet,
	DroppedFrames,
	CpuUsage,
	PreviousScene,
	NextScene,
	InstantReplay,
	BrowserSource,
	RecordToggle,
	StreamToggle,
	SetTransition,
	FilterToggle,
	VirtualCam,
	FocusedWindow,
	SourceVisibility,
	Hotkey,
	Screenshot,
	RecordTools,
	MediaControl,
	ProfileSwitch,
	SetText,
	StreamHealth,
	TransitionDial,
	SceneDial,
	FilterDial,
];
