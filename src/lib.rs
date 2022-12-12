#![no_std]

#[cfg(test)]
extern crate std;

extern crate static_assertions;

pub mod qrwlock;

#[cfg(test)]
mod test {
    use super::qrwlock::*;
    use rand::Rng;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use std::vec::Vec;
    use std::sync::atomic::{Ordering, AtomicU32};

    #[test]
    fn qrwlock_test_single_threaded() {
        let lock = RwLock::new(());

        let locked = lock.write();
        assert!(lock.read_try_lock().is_none());
        assert!(lock.write_try_lock().is_none());
        drop(locked);

        let _locked1 = lock.read();
        let _locked2 = lock.read();

        assert!(lock.write_try_lock().is_none());
    }

    #[test]
    fn qrwlock_test_multy_threaded() {
        const READ_NUM_THREADS: usize = 10;
        const WRITE_NUM_THREADS: usize = 2;
        const WRITER: u32 = 1 << 31;

        let lock = Arc::new(RwLock::new(AtomicU32::new(0)));

        let r_ths: Vec<_> = (0..READ_NUM_THREADS)
            .map(|_| {
                let lock = lock.clone();
                thread::spawn(move || {
                    let mut rng = rand::thread_rng();

                    for _ in 0..100 {
                        let locked = lock.read();
                        assert!((*locked).load(Ordering::Relaxed) & WRITER == 0);

                        (*locked).fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(rng.gen_range(10..50)));
                        (*locked).fetch_sub(1, Ordering::Relaxed);

                        drop(locked);

                        thread::yield_now();
                    }
                })
            })
            .collect();

        let w_ths: Vec<_> = (0..WRITE_NUM_THREADS)
            .map(|_| {
                let lock = lock.clone();
                thread::spawn(move || {
                    let mut rng = rand::thread_rng();

                    for _ in 0..100 {
                        let locked = lock.write();

                        assert!((*locked).compare_exchange(0, WRITER, Ordering::Relaxed, Ordering::Relaxed).is_ok());
                        thread::sleep(Duration::from_millis(rng.gen_range(10..50)));
                        assert!((*locked).compare_exchange(WRITER, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok());

                        drop(locked);

                        thread::yield_now();
                    }
                })
            })
            .collect();

        for th in r_ths {
            th.join().unwrap();
        }

        for th in w_ths {
            th.join().unwrap();
        }
    }
}
