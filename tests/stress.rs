use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, spawn, JoinHandle},
    time::{Duration, SystemTime},
};

use atomic_bus::{AtomicBus, AtomicSubscription};

#[test]
fn single_producer_single_consumer() {
    // setup test
    let context = Arc::new(TestContext::new());
    let bus = Arc::new(AtomicBus::new());
    let producer_messages: Vec<String> = (0..10000).into_iter().map(|x| x.to_string()).collect();
    let producer: TestProducer = TestProducer::new(&context, &bus, &producer_messages);
    let consumer = TestConsumer::new(&context, &bus);

    // execute test
    producer.spawn();
    let consumer_jh = consumer.spawn();
    context.start();
    context.join(1, 1, 10000);
    let consumer_msgs = consumer_jh.join().unwrap();

    // validations
    assert_eq!(10000, consumer_msgs.len());
    for i in 0..10000 {
        assert_eq!(consumer_msgs.get(i).unwrap().as_ref(), &i.to_string());
    }
}

#[test]
fn single_producer_multi_consumer() {
    // setup test
    let context = Arc::new(TestContext::new());
    let bus = Arc::new(AtomicBus::new());
    let producer_messages: Vec<String> = (0..10000).into_iter().map(|x| x.to_string()).collect();
    let producer = TestProducer::new(&context, &bus, &producer_messages);
    let consumer1 = TestConsumer::new(&context, &bus);
    let consumer2 = TestConsumer::new(&context, &bus);

    // execute test
    producer.spawn();
    let consumer1_jh = consumer1.spawn();
    let consumer2_jh = consumer2.spawn();
    context.start();
    context.join(1, 2, 20000);
    let consumer1_msgs = consumer1_jh.join().unwrap();
    let consumer2_msgs = consumer2_jh.join().unwrap();

    // validations
    assert_eq!(10000, consumer1_msgs.len());
    assert_eq!(10000, consumer2_msgs.len());
    for i in 0..10000 {
        assert_eq!(consumer1_msgs.get(i).unwrap().as_ref(), &i.to_string());
        assert_eq!(consumer2_msgs.get(i).unwrap().as_ref(), &i.to_string());
    }
}

#[test]
fn multi_producer_single_consumer() {
    // setup test
    let context = Arc::new(TestContext::new());
    let bus = Arc::new(AtomicBus::new());
    let producer1_messages: Vec<String> = (0..10000)
        .into_iter()
        .map(|x| format!("p1:{x:05}"))
        .collect();
    let producer2_messages: Vec<String> = (0..10000)
        .into_iter()
        .map(|x| format!("p2:{x:05}"))
        .collect();
    let producer3_messages: Vec<String> = (0..10000)
        .into_iter()
        .map(|x| format!("p3:{x:05}"))
        .collect();
    let producer1 = TestProducer::new(&context, &bus, &producer1_messages);
    let producer2 = TestProducer::new(&context, &bus, &producer2_messages);
    let producer3 = TestProducer::new(&context, &bus, &producer3_messages);
    let consumer = TestConsumer::new(&context, &bus);

    // execute test
    producer1.spawn();
    producer2.spawn();
    producer3.spawn();
    let consumer_jh = consumer.spawn();
    context.start();
    context.join(3, 1, 30000);
    let mut consumer_msgs = consumer_jh.join().unwrap();
    consumer_msgs.sort();

    // validations
    assert_eq!(30000, consumer_msgs.len());
    // sorted results, first 10000 will be producer1
    for i in 0..10000 {
        assert_eq!(
            consumer_msgs.get(i).unwrap().as_ref(),
            &format!("p1:{i:05}")
        );
    }
    // sorted results, next 10000 will be producer2
    for i in 0..10000 {
        assert_eq!(
            consumer_msgs.get(10000 + i).unwrap().as_ref(),
            &format!("p2:{i:05}")
        );
    }
    // sorted results, next 10000 will be producer3
    for i in 0..10000 {
        assert_eq!(
            consumer_msgs.get(20000 + i).unwrap().as_ref(),
            &format!("p3:{i:05}")
        );
    }
}

#[test]
fn high_stress() {
    // setup test
    let context = Arc::new(TestContext::new());
    let bus = Arc::new(AtomicBus::new());
    let producer_count = 4;
    let consumer_count = 4;
    let message_count = 100000;

    // start producers and consumers
    for i in 0..producer_count {
        let messages: Vec<String> = (0..message_count)
            .into_iter()
            .map(|x| format!("p{i:03}:{x:09}"))
            .collect();
        TestProducer::new(&context, &bus, &messages).spawn();
    }
    for _ in 0..consumer_count {
        TestConsumer::new(&context, &bus).spawn();
    }

    // execute test, which will verify exact count of messages were received
    context.start();
    context.join(
        producer_count,
        consumer_count,
        producer_count * consumer_count * message_count,
    );
}

struct TestContext {
    start_flag: Arc<AtomicBool>,
    done_flag: Arc<AtomicBool>,
    done_count: Arc<AtomicUsize>,
    consumed_count: Arc<AtomicUsize>,
}
impl TestContext {
    fn new() -> Self {
        Self {
            start_flag: Arc::new(AtomicBool::new(false)),
            done_flag: Arc::new(AtomicBool::new(false)),
            done_count: Arc::new(AtomicUsize::new(0)),
            consumed_count: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn join(&self, producer_count: usize, consumer_count: usize, consumed_count: usize) {
        let started_at = SystemTime::now();
        let timeout_at = started_at + Duration::from_secs(60);

        // wait for producers to be complete
        while SystemTime::now() < timeout_at
            && self.done_count.load(Ordering::Relaxed) < producer_count
        {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(self.done_count.load(Ordering::Relaxed), producer_count);

        // wait for consumer messages to have all been received
        while SystemTime::now() < timeout_at
            && self.consumed_count.load(Ordering::Relaxed) < consumed_count
        {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(self.consumed_count.load(Ordering::Relaxed), consumed_count);

        // set done flag for consumers
        self.done_flag.store(true, Ordering::Relaxed);

        // wait for consumers to be complete
        while SystemTime::now() < timeout_at
            && self.done_count.load(Ordering::Relaxed) < (producer_count + consumer_count)
        {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            self.done_count.load(Ordering::Relaxed),
            producer_count + consumer_count
        );

        // calculate and print rate
        let duration = SystemTime::now().duration_since(started_at).unwrap();
        println!(
            "average rate: {}/sec/consumer",
            (consumed_count as f64
                / (duration.as_micros() as f64 / 1000000.0)
                / consumer_count as f64) as usize
        );
    }
    fn start(&self) {
        self.start_flag.store(true, Ordering::Release);
    }
    fn is_started(&self) -> bool {
        self.start_flag.load(Ordering::Relaxed)
    }
    fn is_done(&self) -> bool {
        self.done_flag.load(Ordering::Relaxed)
    }
    fn increment_consumed_count(&self) {
        self.consumed_count.fetch_add(1, Ordering::Relaxed);
    }
    fn increment_done_count(&self) {
        self.done_count.fetch_add(1, Ordering::Relaxed);
    }
}

struct TestProducer {
    context: Arc<TestContext>,
    bus: Arc<AtomicBus<String>>,
    messages: VecDeque<String>,
}
impl TestProducer {
    fn new(
        context: &Arc<TestContext>,
        bus: &Arc<AtomicBus<String>>,
        messages: &Vec<String>,
    ) -> Self {
        Self {
            context: Arc::clone(context),
            bus: Arc::clone(bus),
            messages: messages.iter().map(|x| x.to_owned()).collect(),
        }
    }
    fn spawn(mut self) {
        spawn(move || {
            while !self.context.is_started() && !self.context.is_done() {
                thread::yield_now();
            }
            while !self.context.is_done() && !self.messages.is_empty() {
                let msg = self.messages.pop_front().unwrap();
                self.bus.send(msg);
            }
            self.context.increment_done_count();
        });
    }
}

struct TestConsumer {
    context: Arc<TestContext>,
    subscription: AtomicSubscription<String>,
    messages: Vec<Arc<String>>,
}
impl TestConsumer {
    fn new(context: &Arc<TestContext>, bus: &Arc<AtomicBus<String>>) -> Self {
        Self {
            context: Arc::clone(context),
            subscription: bus.subscribe(),
            messages: Vec::new(),
        }
    }
    fn spawn(mut self) -> JoinHandle<Vec<Arc<String>>> {
        spawn(move || {
            while !self.context.is_done() {
                match self.subscription.poll() {
                    None => thread::sleep(Duration::from_micros(100)),
                    Some(x) => {
                        self.messages.push(x);
                        self.context.increment_consumed_count();
                    }
                }
            }
            self.context.increment_done_count();
            self.messages
        })
    }
}
