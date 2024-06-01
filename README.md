# atomic-bus

Atomic Bus: An unbounded, lock-free, multi-producer, multi-consumer pub/sub implementation that utilizes atomic operations.

## Examples

### Basic Send/Subscribe

The following example will start a sender thread and printing subscriber thread.

#### Code

```rust
use atomic_bus::AtomicBus;
use std::{sync::Arc, time::Duration, thread};

// create the bus
let bus: AtomicBus<String> = AtomicBus::new();

// subscribing before spawning the sender thread guarantees all sent messages will be received
let mut subscriber = bus.subscribe_mut();

// create and spawn a sender
let sender = bus.create_sender();
let arc_message = Arc::new("all messages are an Arc".to_owned());
thread::spawn(move || {
    sender.send("hello world!".to_owned());
    sender.send(arc_message);
    sender.send("done".to_owned());
});

// spawn printing subscriber and wait for it to complete
thread::spawn(move || loop {
    match subscriber.next() {
        None => thread::sleep(Duration::from_millis(1)),
        Some(x) => {
            println!("subscriber received: {x:?}");
            if x.as_ref() == "done" {
                return;
            }
        }
    }
})
.join()
.unwrap();
```

#### Output

```
subscriber received: "hello world!"
subscriber received: "all messages are an Arc"
subscriber received: "done"
```

### Load Balancing Subscription

The following example will start a sender thread and multiple subscriber threads that shared the
same [`AtomicSubscriber`] in order to load balance received events between multiple threads.
For the sake of this example, the subscriber threads will always sleep after attempting to poll
in order to simulate load and avoid a single greedy consumer receiving all events before others
have had a chance to start.

#### Code

```rust
use atomic_bus::AtomicBus;
use std::{sync::Arc, time::Duration, thread};

// create the bus
let bus: AtomicBus<String> = AtomicBus::new();

// subscribing before spawning the sender thread guarantees all sent messages will be received
let subscriber = Arc::new(bus.subscribe());

// create and spawn a sender
let sender = bus.create_sender();
thread::spawn(move || {
    for i in 0..10 {
        sender.send(format!("message #{i}"));
    }
});

// spawn printing subscriber threads that share a single AtomicSubscription
let mut handles = Vec::new();
{
    for i in 0..3 {
        let subscriber = Arc::clone(&subscriber);
        let handle = thread::spawn(move || loop {
            if let Some(x) = subscriber.next() {
                println!("subscriber {i} received: {x:?}");
                if x.as_ref() == "done" {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(10));
        });
        handles.push(handle);
    }
};
```

#### Output

```
subscriber 0 received: "message #0"
subscriber 1 received: "message #1"
subscriber 2 received: "message #2"
subscriber 0 received: "message #3"
subscriber 1 received: "message #4"
subscriber 2 received: "message #5"
subscriber 0 received: "message #6"
subscriber 1 received: "message #7"
subscriber 2 received: "message #8"
subscriber 0 received: "message #9"
```

License: MIT OR Apache-2.0
