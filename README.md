<p align="center">
  <img width="160" src="https://raw.githubusercontent.com/FeloDeck/obs-tools/main/de.felitendo.obs-tools.sdPlugin/images/plugin.svg" alt="OBS Tools">
</p>

<h1 align="center">OBS Tools for OpenDeck</h1>

<h3 align="center">Control OBS from your Stream Deck on Linux and macOS.</h3>

<p align="center">
  BarRaider's OBS Tools, rebuilt for <a href="https://github.com/nekename/OpenDeck">OpenDeck</a>, plus the everyday OBS actions Linux users miss.
</p>

<h5 align="center">
  <a href="#install">Install</a> |
  <a href="#how-to-use">How to use</a> |
  <a href="#actions">Actions</a> |
  <a href="https://github.com/FeloDeck/obs-tools/issues">Report a bug</a>
</h5>

<p align="center">
  <a href="https://ko-fi.com/felitendo"><img src="https://storage.ko-fi.com/cdn/kofi5.png?v=6" alt="Buy me a coffee on Ko-fi" height="48"></a>
</p>

<p align="center">
  <img width="720" src="https://raw.githubusercontent.com/FeloDeck/obs-tools/main/res/opendeck.png" alt="OpenDeck with OBS Tools keys: live scene pictures, last scene, stream health, record, volume with a level meter, media control, a counter, CPU and four dials, and the settings of a Source Volume key with pictures of the key">
</p>

## Install

You need OpenDeck 2 and OBS 28 or newer, on Linux or macOS with an x86_64 or ARM64 CPU.

In OpenDeck, click **Plugins** and install **OBS Tools** from the **Plugin store**.

<details>
<summary>From source, or a plugin file</summary>

You need [Rust](https://rustup.rs).

```bash
git clone https://github.com/FeloDeck/obs-tools.git
cd obs-tools
make install
```

Then restart OpenDeck. With the OpenDeck Flatpak, install into its folder:

```bash
make install PLUGINS=~/.var/app/me.amankhanna.opendeck/config/opendeck/plugins
```

`make package` builds `dist/de.felitendo.obs-tools.streamDeckPlugin` for x86_64 and ARM64 (it needs zig and cargo-zigbuild). Built on a Mac, the file works on macOS too. You can install that file on the **Plugins** page of OpenDeck.

</details>

## How to use

1. In OBS, open **Tools → WebSocket Server Settings** and turn on **Enable WebSocket server**.
2. In OpenDeck, drag an action from **OBS Tools** onto a key.
3. In the settings of the key, click **Read port and password from OBS**.

That is all when OBS runs on the same PC. For OBS on another PC, click **Change** next to the connection status at the top of any key's settings, and type its address, port and password. The settings apply to all keys.

If something is wrong, the settings tell you what to do, and the keys show **OBS offline** or **Password?**. When a key shows a warning, its settings say why. A key with a yellow dot still needs its settings.

## Actions

Everything from BarRaider's OBS Tools, plus what Elgato's own OBS plugin does on Windows and macOS.

<details>
<summary>All 32 actions</summary>

| Action | What it does |
|---|---|
| Recording Toggle | Start, stop or pause recording. Shows the time. |
| Streaming Toggle | Start or stop streaming. Shows the time. |
| Smart Scene Switcher | Switch scenes, with a live picture. In studio mode: first press to preview, second to go live. |
| Source Mute Toggle | Mute or unmute an audio source, or push to talk. |
| Source Visibility | Show or hide a source. |
| Instant Replay | Save the replay buffer and play it on stream, in slow motion if you like. Hold to turn the buffer on or off. |
| Source Volume | Volume up, down or to a value, with a live level meter. Hold to keep going. |
| Volume (Dial) | Turn for volume, press or touch to mute. |
| Scene (Dial) | Turn to pick a scene (with a picture), press to switch. |
| Studio Mode Toggle | Turn studio mode on or off, or take the preview live. |
| Last Scene | Back to the scene that was live before. |
| Next Scene | The next scene in the list, or the one above. |
| Media Control | Play, pause, stop, restart or jump in a media source. Shows the time left or played. |
| Video Player | Play a file in a media source. Many keys can share one source. |
| Image Source | Show a picture in an image source, and hide it again after some seconds. |
| Set Text | Set a text, go through a list, count up or down (`Deaths: {n}`), or run a timer (`Live in {t}`). Press, hold and double press can each do any of these. |
| Filter Toggle | Turn a filter on or off. |
| Stream Health | Bitrate, dropped frames and a green, yellow or red light. |
| Virtual Camera Toggle | Start or stop the virtual camera. |
| Recording Marker | Add a chapter marker or start a new file while recording. |
| Screenshot | Save a picture of the live scene, or of any scene or source. |
| OBS Hotkey | Press any OBS hotkey, by name or by its keys. |
| Set Transition | Set the transition and its duration. |
| Transition (Dial) | Turn to change the transition time, press for the next one. |
| Filter (Dial) | Turn to change a number in a filter, like gain or opacity. |
| Source Animation | Move, scale, rotate or crop a source in phases. Record them in OBS, export and import them as text. |
| Browser Source | Refresh, set a URL, go through a list of URLs, show or hide. |
| Dropped Frames Alarm | Flashes when frames get dropped, skipped or missed. |
| OBS CPU Usage | How much CPU OBS uses. Turns red above a limit. |
| Source Monitor Set | Audio monitoring off, monitor only, or monitor and output. |
| Profile / Scene Collection | Switch to another profile or scene collection. |
| Focused Window Capture | Point a window capture at the window that has focus (on Linux, OBS on X11). |

Most keys can do one thing on press, another on hold and another on double press. Keys that turn something on or off can also only turn it on, or only off. That helps in multi actions. Many keys can show the name of their source as title.

</details>

## Credits

To [BarRaider](https://barraider.com) for the original OBS Tools, and to the OpenDeck developers.

GPL-3.0-or-later.