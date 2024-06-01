//! Atomic Bus: An unbounded, lock-free, multi-producer, multi-consumer pub/sub implementation that utilizes atomic operations.
//!
//! # Examples
//!
//! ## Basic Send/Subscribe
//!
//! The following example will start a sender thread and printing subscriber thread.
//!
//! ### Code
//!
//! ```
//! use atomic_bus::AtomicBus;
//! use std::{sync::Arc, time::Duration, thread};
//!
//! // create the bus
//! let bus: AtomicBus<String> = AtomicBus::new();
//!
//! // subscribing before spawning the sender thread guarantees all sent messages will be received
//! let mut subscriber = bus.subscribe_mut();
//!
//! // create and spawn a sender
//! let sender = bus.create_sender();
//! let arc_message = Arc::new("all messages are an Arc".to_owned());
//! thread::spawn(move || {
//!     sender.send("hello world!".to_owned());
//!     sender.send(arc_message);
//!     sender.send("done".to_owned());
//! });
//!
//! // spawn printing subscriber and wait for it to complete
//! thread::spawn(move || loop {
//!     match subscriber.poll() {
//!         None => thread::sleep(Duration::from_millis(1)),
//!         Some(x) => {
//!             println!("subscriber received: {x:?}");
//!             if x.as_ref() == "done" {
//!                 return;
//!             }
//!         }
//!     }
//! })
//! .join()
//! .unwrap();
//! ```
//!
//! ### Output
//!
//! ```text
//! subscriber received: "hello world!"
//! subscriber received: "all messages are an Arc"
//! subscriber received: "done"
//! ```
//!
//! ## Load Balancing Subscription
//!
//! The following example will start a sender thread and multiple subscriber threads that shared the
//! same [`AtomicSubscriber`] in order to load balance received events between multiple threads.
//! For the sake of this example, the subscriber threads will always sleep after attempting to poll
//! in order to simulate load and avoid a single greedy consumer receiving all events before others
//! have had a chance to start.
//!
//! ### Code
//!
//! ```
//! use atomic_bus::AtomicBus;
//! use std::{sync::Arc, time::Duration, thread};
//!
//! // create the bus
//! let bus: AtomicBus<String> = AtomicBus::new();
//!
//! // subscribing before spawning the sender thread guarantees all sent messages will be received
//! let subscriber = Arc::new(bus.subscribe());
//!
//! // create and spawn a sender
//! let sender = bus.create_sender();
//! thread::spawn(move || {
//!     for i in 0..10 {
//!         sender.send(format!("message #{i}"));
//!     }
//! });
//!
//! // spawn printing subscriber threads that share a single AtomicSubscription
//! let mut handles = Vec::new();
//! {
//!     for i in 0..3 {
//!         let subscriber = Arc::clone(&subscriber);
//!         let handle = thread::spawn(move || loop {
//!             if let Some(x) = subscriber.poll() {
//!                 println!("subscriber {i} received: {x:?}");
//!                 if x.as_ref() == "done" {
//!                     return;
//!                 }
//!             }
//!             thread::sleep(Duration::from_millis(10));
//!         });
//!         handles.push(handle);
//!     }
//! };
//! ```
//!
//! ### Output
//!
//! ```text
//! subscriber 0 received: "message #0"
//! subscriber 1 received: "message #1"
//! subscriber 2 received: "message #2"
//! subscriber 0 received: "message #3"
//! subscriber 1 received: "message #4"
//! subscriber 2 received: "message #5"
//! subscriber 0 received: "message #6"
//! subscriber 1 received: "message #7"
//! subscriber 2 received: "message #8"
//! subscriber 0 received: "message #9"
//! ```

use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};

/// A bus, which can send data to a set of active subscriptions.
///
/// # Data Lifetime
///
/// All underlying data is wrapped in an `Arc` which will be dropped immediately after all active subscribers have dropped their respective references.
///
/// # Sync
///
/// `AtomicBus` is [`Sync`], so it can be wrapped in an [`Arc`] to safely share between threads.
pub struct AtomicBus<T> {
    end: Arc<ArcSwap<NodeLink<T>>>,
}
impl<T> AtomicBus<T> {
    /// Create a new `AtomicBus`
    pub fn new() -> Self {
        Self {
            end: Arc::new(ArcSwap::new(Arc::new(NodeLink::default()))),
        }
    }

    /// Send an event to the [`AtomicBus`] which will be received by all current subscribers.
    ///
    /// # Alternatives
    ///
    /// If you wish to limit code to be publish-only, see [`AtomicBus::sender`] to create a send-only struct for this bus.
    pub fn send<A: Into<Arc<T>>>(&self, data: A) {
        append_tail(self.end.as_ref(), data.into());
    }

    /// Create a send-only structure to produce messages to this bus.
    pub fn create_sender(&self) -> AtomicSender<T> {
        AtomicSender::new(Arc::clone(&self.end))
    }

    /// Create an atomic subscription to this bus, which is [`Sync`] and can be shared between threads.
    pub fn subscribe(&self) -> AtomicSubscription<T> {
        AtomicSubscription::new(&self.end)
    }

    /// Create a mutable subscription to this bus, which is slightly faster than the atomic variant but is not [`Sync`].
    pub fn subscribe_mut(&self) -> MutSubscription<T> {
        MutSubscription::new(&self.end)
    }
}
impl<T> Default for AtomicBus<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The send side of an [`AtomicBus`], providing identical functionality to [`AtomicBus::send`], but without the ability to create new subscribers.
///
/// # Clone
///
/// Cloning an [`AtomicSender`] will result in a new sender that is able to send on the same underlying [`AtomicBus`].
#[derive(Clone)]
pub struct AtomicSender<T> {
    end: Arc<ArcSwap<NodeLink<T>>>,
}
impl<T> AtomicSender<T> {
    fn new(end: Arc<ArcSwap<NodeLink<T>>>) -> Self {
        Self { end }
    }
    /// See [`AtomicBus::send`]
    pub fn send<A: Into<Arc<T>>>(&self, data: A) {
        append_tail(self.end.as_ref(), data.into());
    }
}

fn append_tail<T>(end: &ArcSwap<NodeLink<T>>, data: Arc<T>) {
    let node = Arc::new(Node::new(data.into()));
    let new_end = Arc::clone(&node.link);
    let old_end = end.swap(new_end);
    old_end.next.store(Some(node));
}

/// A [`Sync`] subscription that can be shared between threads, where each message will be delivered exactly once.
///
/// # Clone
///
/// Cloning an `AtomicSubscription` will create a new subscription at an identical position to the original.
/// In other words, if there are 10 pending events to consume and the subscription is cloned, the cloned subscription
/// will also have 10 pending events.
///
/// **Note:** This behavior means that cloning an [`AtomicSubscription`] results in different behavior than cloning
/// an [`Arc<AtomicSubscription>`]. The former will create a new subscription at the same point in the data stream,
/// while the latter will poll from the same subscription to share the load.
///
/// # Performance
///
/// The performance will be slightly worse than [`MutSubscription`], but allows for multiple workers to share the load from
/// multiple threads by taking messages from a shared subscription as they are available.
pub struct AtomicSubscription<T> {
    position: Arc<ArcSwap<NodeLink<T>>>,
}
impl<T> AtomicSubscription<T> {
    fn new(position: &ArcSwap<NodeLink<T>>) -> Self {
        Self {
            position: Arc::new(ArcSwap::new(position.load_full())),
        }
    }
    /// Poll the next message for this subscription, returning immediately with [`None`] if no new messages were available.
    pub fn poll(&self) -> Option<Arc<T>> {
        let mut data = None;
        self.position
            .rcu(|old_position| match old_position.next.load_full() {
                None => {
                    // maintain last position
                    data = None;
                    Arc::clone(old_position)
                }
                Some(x) => {
                    // advance position
                    let new_position = Arc::clone(&x.link);
                    data = Some(x);
                    new_position
                }
            });
        data.map(|x| Arc::clone(&x.data))
    }
}
impl<T> Clone for AtomicSubscription<T> {
    fn clone(&self) -> Self {
        Self::new(self.position.as_ref())
    }
}

/// A mutable subscription that utilizes `&mut self` to track the current position.
///
/// # Clone
///
/// Cloning a `MutSubscription` will create a new subscription at an identical position to the original.
/// In other words, if there are 10 pending events to consume and the subscription is cloned, the cloned subscription
/// will also have 10 pending events.
///
/// # Performance
///
/// While providing identical functionality to [`AtomicSubscription`], it is slightly faster at the expense of not being [`Sync`].
#[derive(Clone)]
pub struct MutSubscription<T> {
    position: Arc<NodeLink<T>>,
}
impl<T> MutSubscription<T> {
    fn new(position: &ArcSwap<NodeLink<T>>) -> Self {
        Self {
            position: position.load_full(),
        }
    }
    /// Poll the next message for this subscription, returning immediately with [`None`] if no new messages were available.
    pub fn poll(&mut self) -> Option<Arc<T>> {
        match self.position.next.load_full() {
            None => None,
            Some(next) => {
                let data = Arc::clone(&next.data);
                self.position = Arc::clone(&next.link);
                Some(data)
            }
        }
    }
}
impl<T> Iterator for MutSubscription<T> {
    type Item = Arc<T>;
    fn next(&mut self) -> Option<Self::Item> {
        self.poll()
    }
}

struct Node<T> {
    pub data: Arc<T>,
    pub link: Arc<NodeLink<T>>,
}
impl<T> Node<T> {
    fn new(data: Arc<T>) -> Self {
        Self {
            data,
            link: Arc::new(NodeLink::default()),
        }
    }
}

struct NodeLink<T> {
    pub next: ArcSwapOption<Node<T>>,
}
impl<T> Default for NodeLink<T> {
    fn default() -> Self {
        Self {
            next: ArcSwapOption::new(None),
        }
    }
}
