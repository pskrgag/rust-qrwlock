//! A fair rwlock. Enspired by [qrwlock from linux](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/include/asm-generic/qrwlock.h) 

use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU32, AtomicU8, Ordering},
};
use spin::{mutex::TicketMutex, relax::Spin, RelaxStrategy};

const WRITER_LOCKED: u32 = 0xff;
const WRITER_WAITING: u32 = 1 << 8;
const WRITER_MASK: u32 = WRITER_LOCKED | WRITER_WAITING;
const READER_COUNT: u32 = 1 << 9;

#[cfg(target_endian = "big")]
#[repr(C)]
struct RawRwLockBits {
    padd: [u8; 3],
    w_lock: ManuallyDrop<AtomicU8>,
}

#[cfg(target_endian = "little")]
#[repr(C)]
struct RawRwLockBits {
    w_lock: ManuallyDrop<AtomicU8>,
    padd: [u8; 3],
}

#[repr(C)]
union RawRwlock {
    bits: ManuallyDrop<AtomicU32>,
    raw: ManuallyDrop<RawRwLockBits>,
}

static_assertions::const_assert!(core::mem::size_of::<RawRwlock>() == core::mem::size_of::<u32>());

/// A [read-write lock](https://en.wikipedia.org/wiki/Readers%E2%80%93writer_lock) that provides
/// serialization between one reader and any amount of readers
///
/// This lock acts as `std::sync::RwLock`, but uses spinning as backoff mechanism and does not
/// prone to reader or writer starvations
///
/// Why not generic rwlock?
///
/// Generic rwlock has 2 types: read-preffered and write-preffered. That means
/// readers or writters should wait for all opposite accessors finish their work.
/// This might cause denial of service problems when lock is under huge contention or
/// in AMP enviroment.
///
/// Qrwlock solves unfairness by serializing lock request with FIFO waitqueue based on
/// ticket spinlock. That means if writer was placed into waitqueue before a reader,
/// its lock request would be served earlier.
pub struct RwLock<T> {
    raw: RawRwlock,
    data: UnsafeCell<T>,
    wq: TicketMutex<()>,
}

/// Guard that provides read-only access to underlying data
pub struct ReadGuard<'a, T: 'a> {
    lock: &'a RwLock<T>,
    data: &'a T,
}

/// Guard that provides read-rite access to underlying data
/// WriteGuard<T> guarantees exclusive access.
pub struct WriteGuard<'a, T: 'a> {
    lock: &'a RwLock<T>,
    data: &'a mut T,
}

impl<T> RwLock<T> {
    /// Creates a new rwlock wrapping passed data
    #[inline]
    pub fn new(data: T) -> Self {
        Self {
            wq: TicketMutex::new(()),
            raw: unsafe { core::mem::zeroed() },
            data: UnsafeCell::new(data),
        }
    }

    /// Tries to acquire the lock for write.
    ///
    /// Returns Some(WriteGuard<T>) if lock was acquired, None otherwise
    #[inline(always)]
    pub fn write_try_lock(&self) -> Option<WriteGuard<T>> {
        let raw = self.raw(Ordering::Relaxed);

        if raw == 0
            && unsafe {
                self.raw
                    .bits
                    .compare_exchange(0, WRITER_LOCKED, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
            }
        {
            Some(WriteGuard {
                lock: &self,
                data: unsafe { &mut *self.data.get() },
            })
        } else {
            None
        }
    }

    /// Tries to acquire the lock for read
    ///
    /// Returns Some(ReadGuard<T>) if lock was acquired, None otherwise
    #[inline(always)]
    pub fn read_try_lock(&self) -> Option<ReadGuard<T>> {
        let mut raw = self.raw(Ordering::Relaxed);

        if raw & WRITER_MASK == 0 {
            raw = self.add_read_count(Ordering::Acquire);
            if raw & WRITER_MASK == 0 {
                Some(ReadGuard {
                    lock: &self,
                    data: unsafe { &*self.data.get() },
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    fn wait_for_writes_to_unlock(&self) {
        loop {
            let cur = self.raw(Ordering::Acquire);

            if cur & WRITER_MASK == 0 {
                break;
            }

            Spin::relax();
        }
    }

    pub(crate) fn raw(&self, order: Ordering) -> u32 {
        unsafe { self.raw.bits.load(order) }
    }

    #[inline(always)]
    fn add_read_count(&self, order: Ordering) -> u32 {
        unsafe { self.raw.bits.fetch_add(READER_COUNT, order) }
    }

    #[inline(always)]
    pub(crate) fn sub_read_count(&self, order: Ordering) -> u32 {
        unsafe { self.raw.bits.fetch_sub(READER_COUNT, order) }
    }

    #[inline(always)]
    fn read_lock_fast(&self) -> bool {
        let state = self.add_read_count(Ordering::Acquire);

        if (state & WRITER_MASK) == 0 {
            true
        } else {
            // Here we just maintain the counter, so no semantics are needed
            self.sub_read_count(Ordering::Relaxed);
            false
        }
    }

    #[inline(always)]
    fn write_lock_fast(&self) -> bool {
        unsafe {
            self.raw
                .bits
                .compare_exchange(0, WRITER_LOCKED, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        }
    }

    fn read_lock_slow(&self) {
        // Imaginary value to force drop at the end of the function
        let _guard = self.wq.lock();

        // Here we just maintain the counter, so no semantics are needed
        self.add_read_count(Ordering::Relaxed);

        self.wait_for_writes_to_unlock();
    }

    fn write_lock_slow(&self) {
        let _guard = self.wq.lock();

        if self.raw(Ordering::Relaxed) == 0 && self.write_lock_fast() {
            return;
        }

        unsafe { self.raw.bits.fetch_or(WRITER_WAITING, Ordering::Relaxed) };

        loop {
            let raw = self.raw(Ordering::Relaxed);

            if raw == WRITER_WAITING
                && unsafe {
                    self.raw
                        .bits
                        .compare_exchange(
                            WRITER_WAITING,
                            WRITER_LOCKED,
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                }
            {
                return;
            }

            Spin::relax();
        }
    }

    /// Acquire the lock for read
    ///
    /// Returns ReadGuard<T>
    ///
    /// If lock is locked for readers than only readers may access the underlying data
    /// This function is divided into fast and slow path. Fast path is inlined, slow path is not
    #[inline(always)]
    pub fn read(&self) -> ReadGuard<T> {
        if !self.read_lock_fast() {
            self.read_lock_slow();
        }

        ReadGuard {
            lock: &self,
            data: unsafe { &*self.data.get() },
        }
    }

    /// Acquire the lock for write
    ///
    /// Returns WriteGuard<T>
    ///
    /// This function is divided into fast and slow path. Fast path is inlined, slow path is not
    #[inline(always)]
    pub fn write(&self) -> WriteGuard<T> {
        if !self.write_lock_fast() {
            self.write_lock_slow();
        }

        WriteGuard {
            lock: &self,
            data: unsafe { &mut *self.data.get() },
        }
    }

    #[inline(always)]
    pub(crate) fn write_unlock(&self) {
        unsafe {
            self.raw.raw.w_lock.store(0, Ordering::Release)
        };
    }
}

impl<'a, T> Drop for ReadGuard<'a, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.lock.sub_read_count(Ordering::Release);
    }
}

impl<'a, T> Drop for WriteGuard<'a, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, T> Deref for WriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, T> DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

unsafe impl<T> Sync for RwLock<T> {}
unsafe impl<T> Send for RwLock<T> {}
