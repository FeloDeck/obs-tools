// What a key shows: its image, title, state and touch strip, the setup image and the status image.
// Sends to OpenDeck only what changed.
use serde_json::Value;

use super::Key;
use crate::actions::Controller;
use crate::icons::colors;
use crate::js;
use crate::key_images::key_image;
use crate::obs::Status;
use crate::render::{GlyphKey, glyph_key, status_key, status_label};
use crate::timers::set_timeout;

impl Key {
	// ----- setup and status images -----

	pub(super) fn show_status_image(&self) {
		let Some(status) = self.obs().map(|c| c.status()) else { return };
		if !self.visible() || matches!(status, Status::Idle | Status::Connecting) {
			return;
		}
		self.b.borrow_mut().screen.status_shown = Some(status);
		// Dials show it as text in their offline look (see dial_status).
		if self.controller() == Controller::Encoder {
			if let Some(me) = self.rc() {
				self.behavior().render_offline(&me);
			}
		} else {
			self.send_image(&status_key(status), true);
		}
	}

	pub(super) fn clear_status_image(&self) {
		if self.b.borrow_mut().screen.status_shown.take().is_none() {
			return;
		}
		// Dials draw again once they are online.
		if self.controller() == Controller::Encoder {
			return;
		}
		if self.b.borrow().screen.setup_shown {
			self.send_image(&self.setup_image(), true);
		} else {
			self.reset_image(false);
		}
	}

	/// What a dial shows instead of its value while OBS cannot be reached: the words and colors
	/// of the status image of a key. Empty while connecting.
	pub fn dial_status(&self) -> Value {
		let Some(status) = self.b.borrow().screen.status_shown else { return Value::from("") };
		crate::obj! { "value" => status_label(status), "color" => if status == Status::Offline { colors::MUTED } else { colors::AMBER } }
	}

	fn icon(&self) -> String {
		crate::manifest::action_def(self.kind.uuid).map_or_else(|| "plugin".to_string(), |a| a.icon.to_string())
	}

	/// The glyph of the action in gray with a yellow dot: "pick something first".
	pub fn setup_image(&self) -> String {
		glyph_key(&self.icon(), &GlyphKey { color: colors::GRAY, setup: true, ..Default::default() })
	}

	/// Draw the key, or the setup image while it is not set up. Dials show this in their feedback.
	pub fn draw(&self) {
		if self.show_setup() {
			return;
		}
		if let Some(me) = self.rc() {
			self.behavior().render(&me);
		}
	}

	pub fn draw_offline(&self) {
		if self.show_setup() {
			return;
		}
		if let Some(me) = self.rc() {
			self.behavior().render_offline(&me);
		}
	}

	fn show_setup(&self) -> bool {
		if self.controller() != Controller::Keypad {
			return false;
		}
		if !self.is_set_up() {
			self.b.borrow_mut().screen.setup_shown = true;
			if self.b.borrow().screen.status_shown.is_none() {
				self.send_image(&self.setup_image(), true);
			}
			return true;
		}
		if self.b.borrow().screen.setup_shown {
			self.b.borrow_mut().screen.setup_shown = false;
			self.reset_image(true);
		}
		false
	}

	// ----- keys -----

	/// A key image from the manifest, drawn again with a title under the glyph.
	pub fn state_image(&self, state: u16, title: &str) -> String {
		let def = crate::manifest::action_def(self.kind.uuid);
		let name = def.and_then(|a| a.state_images().get(state as usize).cloned());
		let (glyph, color) = match name.as_deref().and_then(key_image) {
			Some(image) => (image.glyph.to_string(), image.color.unwrap_or(colors::WHITE)),
			None => (self.icon(), colors::WHITE),
		};
		glyph_key(&glyph, &GlyphKey { color, title, ..Default::default() })
	}

	/// The name of the source (or filter) as title, on keys whose pictures come from the manifest.
	pub fn show_name_title(&self, name: &str) {
		self.glyph_for_title(if self.s_truthy("showName") && !name.is_empty() { name } else { "" });
	}

	/// The manifest pictures have the glyph in the middle. With a title, draw it with the title below.
	pub fn glyph_for_title(&self, title: &str) {
		self.own_title(title);
		if !title.is_empty() {
			self.set_image(&self.state_image(self.state(), title));
		} else {
			self.reset_image(false);
		}
	}

	fn set_title(&self, title: &str) {
		if !self.visible() {
			return;
		}
		{
			let mut b = self.b.borrow_mut();
			let screen = &mut b.screen;
			if screen.title.as_deref() == Some(title) {
				return;
			}
			screen.title = Some(title.to_string());
		}
		self.sd.set_title(&self.context, title);
	}

	/// Keys draw their titles themselves, close to the glyph. A title in OpenDeck would be a second
	/// one (older versions set it), so it is cleared while the key shows its own. Without one, a title
	/// the user typed stays.
	pub fn own_title(&self, title: &str) {
		if !title.is_empty() {
			self.set_title("");
		}
	}

	pub fn set_image(&self, image: &str) {
		self.send_image(image, false);
	}

	/// force: also while the setup or status image is shown.
	fn send_image(&self, image: &str, force: bool) {
		if !self.visible() {
			return;
		}
		{
			let mut b = self.b.borrow_mut();
			let screen = &mut b.screen;
			if !force && (screen.setup_shown || screen.status_shown.is_some()) {
				return;
			}
			if screen.image.as_deref() == Some(image) {
				return;
			}
			screen.image = Some(image.to_string());
			screen.image_overridden = true;
		}
		self.sd.set_image(&self.context, image);
	}

	/// Back to the image from the manifest (or the one the user picked).
	pub fn reset_image(&self, force: bool) {
		{
			let mut b = self.b.borrow_mut();
			let screen = &mut b.screen;
			if !force && (screen.setup_shown || screen.status_shown.is_some()) {
				return;
			}
			if !screen.image_overridden {
				return;
			}
			screen.image_overridden = false;
			screen.image = None;
		}
		if self.visible() {
			self.sd.set_image(&self.context, "");
		}
	}

	pub fn set_state(&self, state: u16) {
		{
			let mut b = self.b.borrow_mut();
			if !b.visible && !b.is_in_multi_action {
				return;
			}
			if b.state == state && b.screen.state == Some(state) {
				return;
			}
			b.state = state;
			b.screen.state = Some(state);
		}
		self.sd.set_state(&self.context, state);
	}

	pub fn show_ok(&self) {
		if self.visible() {
			self.sd.show_ok(&self.context);
		}
	}

	pub fn show_alert(&self) {
		if self.visible() {
			self.sd.show_alert(&self.context);
		}
	}

	pub fn send_to_pi(&self, payload: Value) {
		self.sd.send_to_property_inspector(&self.context, payload);
	}

	// ----- dials -----

	pub fn set_layout(&self) {
		let Some(me) = self.rc() else { return };
		let layout = if self.controller() == Controller::Encoder { self.behavior().layout(&me) } else { None };
		let Some(layout) = layout else { return };
		self.b.borrow_mut().screen.feedback = None;
		self.sd.set_feedback_layout(&self.context, layout);
		self.resend_feedback_soon();
	}

	/// OpenDeck loads a custom touch strip layout again after the key appeared, which drops what
	/// was sent before. So send it again a little later.
	fn resend_feedback_soon(&self) {
		for ms in [700.0, 2500.0] {
			let me = self.me.clone();
			let id = self.timer_id();
			let timer = set_timeout(ms, move || {
				let Some(me) = me.upgrade() else { return };
				me.b.borrow_mut().timers.remove(&id);
				if !me.visible() {
					return;
				}
				me.b.borrow_mut().screen.feedback = None;
				let behavior = me.behavior();
				if me.online() {
					behavior.render(&me);
				} else {
					behavior.render_offline(&me);
				}
			});
			self.b.borrow_mut().timers.insert(id, timer);
		}
	}

	pub fn set_feedback(&self, payload: Value) {
		if !self.visible() {
			return;
		}
		let text = js::stringify(&payload);
		{
			let mut b = self.b.borrow_mut();
			let screen = &mut b.screen;
			if screen.feedback.as_ref() == Some(&text) {
				return;
			}
			screen.feedback = Some(text);
		}
		self.sd.set_feedback(&self.context, payload);
	}
}
