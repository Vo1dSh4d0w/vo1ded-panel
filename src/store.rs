//! A tiny observable value, used to share service state with widgets.

use std::cell::{Ref, RefCell};
use std::rc::Rc;

type Subscriber<T> = Box<dyn Fn(&T) -> bool>;

pub struct Store<T> {
    value: RefCell<T>,
    subscribers: RefCell<Vec<Subscriber<T>>>,
}

impl<T: 'static> Store<T> {
    pub fn new(value: T) -> Rc<Self> {
        Rc::new(Self {
            value: RefCell::new(value),
            subscribers: RefCell::new(Vec::new()),
        })
    }

    pub fn get(&self) -> Ref<'_, T> {
        self.value.borrow()
    }

    /// Calls `f` now and on every change. `f` returns `false` to unsubscribe
    /// (usually because the widget it updates has been destroyed).
    pub fn subscribe(&self, f: impl Fn(&T) -> bool + 'static) {
        if f(&self.value.borrow()) {
            self.subscribers.borrow_mut().push(Box::new(f));
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.value.borrow_mut());
        self.notify();
    }

    /// Replaces the value, notifying only when it actually changed.
    pub fn set(&self, value: T)
    where
        T: PartialEq,
    {
        if *self.value.borrow() != value {
            self.update(|v| *v = value);
        }
    }

    fn notify(&self) {
        // Subscribers may subscribe new callbacks while running, so the list
        // is taken out while iterating and merged back afterwards.
        let mut subs = std::mem::take(&mut *self.subscribers.borrow_mut());
        {
            let value = self.value.borrow();
            subs.retain(|s| s(&value));
        }
        let mut current = self.subscribers.borrow_mut();
        subs.append(&mut current);
        *current = subs;
    }
}
