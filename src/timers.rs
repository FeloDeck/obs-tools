// setTimeout and setInterval. A callback runs as its own task, so clearing a timer never stops a
// callback that already started, like in JavaScript.
use std::time::Duration;

use tokio::task::{AbortHandle, spawn_local};

#[derive(Debug)]
pub struct Timer(AbortHandle);

impl Timer {
	pub fn clear(&self) {
		self.0.abort();
	}
}

fn duration(ms: f64) -> Duration {
	// Like Node: at least 1 ms.
	Duration::from_secs_f64(if ms.is_finite() && ms >= 1.0 { ms } else { 1.0 } / 1000.0)
}

pub fn set_timeout(ms: f64, callback: impl FnOnce() + 'static) -> Timer {
	let handle = spawn_local(async move {
		tokio::time::sleep(duration(ms)).await;
		// A task of its own: clear() after this point does not cancel the callback.
		spawn_local(async move { callback() });
	});
	Timer(handle.abort_handle())
}

pub fn set_interval(ms: f64, callback: impl Fn() + 'static) -> Timer {
	let callback = std::rc::Rc::new(callback);
	let handle = spawn_local(async move {
		loop {
			tokio::time::sleep(duration(ms)).await;
			let callback = callback.clone();
			spawn_local(async move { callback() });
		}
	});
	Timer(handle.abort_handle())
}

/// await new Promise((r) => setTimeout(r, ms))
pub async fn sleep(ms: f64) {
	tokio::time::sleep(duration(ms)).await;
}
