// A small event emitter: listeners run right away, in the order they were added.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Listener<T> = Rc<dyn Fn(&T)>;

pub struct Emitter<T: ?Sized> {
	next: Cell<u64>,
	listeners: RefCell<Vec<(u64, Listener<T>)>>,
}

impl<T: ?Sized> Default for Emitter<T> {
	fn default() -> Self {
		Emitter { next: Cell::new(1), listeners: RefCell::new(Vec::new()) }
	}
}

impl<T: ?Sized> Emitter<T> {
	pub fn on(&self, listener: impl Fn(&T) + 'static) -> u64 {
		let id = self.next.get();
		self.next.set(id + 1);
		self.listeners.borrow_mut().push((id, Rc::new(listener)));
		id
	}

	pub fn off(&self, id: u64) {
		self.listeners.borrow_mut().retain(|(i, _)| *i != id);
	}

	/// The listeners at the time of the call all run, even if one removes another.
	pub fn emit(&self, value: &T) {
		let listeners: Vec<Listener<T>> = self.listeners.borrow().iter().map(|(_, l)| l.clone()).collect();
		for listener in listeners {
			listener(value);
		}
	}

	pub fn count(&self) -> usize {
		self.listeners.borrow().len()
	}
}
