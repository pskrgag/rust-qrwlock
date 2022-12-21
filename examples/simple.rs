extern crate qrwlock;
use std::{sync::Arc, thread};

fn main() {
    let counter = Arc::new(qrwlock::RwLock::new(0));

    let thread = thread::spawn({
        let counter = counter.clone();
        move || {
            for _ in 0..1000 {
                *counter.write() += 1;
            }
        }
    });

    for _ in 0..1000 {
        println!("read {}", *counter.read());
    }

    thread.join().unwrap();

    assert_eq!(*counter.read(), 1000);
}
