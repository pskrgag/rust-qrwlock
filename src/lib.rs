#![no_std]

#[cfg(test)]
extern crate std;

#[macro_use]
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
        const WRITE_LOCK: usize = 1 << 31;

        let lock = Arc::new(RwLock::new(0));
        let r_ths: Vec<_> = (0..READ_NUM_THREADS)
            .map(|_| {
                let lock = lock.clone();
                thread::spawn(move || {
                    let mut rng = rand::thread_rng();

                    for _ in 0..100 {
                        let locked = lock.read();
                        assert!(*locked & WRITE_LOCK == 0);
                        thread::sleep(Duration::from_millis(rng.gen_range(10..20)));
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
                        let mut locked = lock.write();
                        assert!(*locked & WRITE_LOCK == 0);
                        *locked |= WRITE_LOCK;
                        thread::sleep(Duration::from_millis(rng.gen_range(10..20)));
                        *locked &= !WRITE_LOCK;
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
