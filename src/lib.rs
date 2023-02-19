//! This crates provides a sequentially locking Ring Buffer. It allows for
//! a fast and non-writer-blocking SPMC-queue, where all consumers read all
//! messages.
//!
//! # Original
//! This is the Rust-wrapped Zig version of the [sling](https://crates.io/crates/sling) crate. Note
//! that due to current constraints in the implementation, the buffer size is set to 256 and the
//! and messages are set to [u8; 8].
//!
//! # Usage
//!
//! There are two ways of consuming from the queue. If threads share a
//! [`SharedReader`] through a shared reference, they will steal
//! queue items from one anothers such that no two threads will read the
//! same message. When a [`SharedReader`] is cloned, the new
//! [`SharedReader`]'s reading progress will no longer affect the other
//! one. If two threads each use a separate [`SharedReader`], they
//! will be able to read the same messages.
//!
//! ```rust
//! # use zsling::*;
//!
//! let buffer = RingBuffer::new();
//!
//! let mut writer = buffer.try_lock().unwrap();
//! let mut reader = buffer.reader();
//!
//! std::thread::scope(|s| {
//!     let reader = &reader;
//!     for t in 0..8 {
//!         s.spawn(move || {
//!             for _ in 0..100 {
//!                 if let Some(val) = reader.pop_front() {
//!                     println!("t: {}, val: {:?}", t, val);
//!                 };
//!             }
//!         });
//!     }
//!
//!     for i in 0..100 {
//!         writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
//!     }
//! });
//! ```
//! # Important!
//!
//! It is also important to keep in mind, that slow readers will be overrun by the writer if they
//! do not consume messages quickly enough. This can happen quite frequently if the buffer size is
//! not large enough. It is advisable to test applications on a case-by-case basis and find a
//! buffer size that is optimal to your use-case.

#![warn(missing_docs)]
#![no_std]

/// A fixed-size, non-write-blocking, ring buffer, that behaves like a
/// SPMC queue and can be safely shared across threads.
/// It is limited to only work for types that are copy, as multiple
/// threads can read the same message.
#[derive(Debug)]
#[repr(C)]
pub struct RingBuffer {
    index: Padded<usize>,
    version: Padded<usize>,
    locked: Padded<bool>,
    data: [Block; 256],
}

#[derive(Debug)]
#[repr(C)]
#[repr(align(128))]
struct Padded<T> {
    data: T,
}

#[derive(Debug)]
#[repr(C)]
struct Block {
    version: usize,
    message: [u8; 8],
}

/// Provides exclusive write access to the [`RingBuffer`].
#[derive(Debug)]
#[repr(C)]
pub struct WriteGuard {
    buffer: *mut RingBuffer,
}

/// Shared read access to its buffer. When multiple threads consume from the
/// [`RingBuffer`] throught the same [`SharedReader`], they will share progress
/// on the queue. Distinct [`RingBuffers`] do not share progress.
#[derive(Debug)]
#[repr(C)]
pub struct SharedReader {
    buffer: *mut RingBuffer,
    index: Padded<usize>,
    version: Padded<usize>,
}

#[allow(dead_code)]
#[repr(u32)]
enum Tag {
    Success,
    Error,
}

#[repr(C)]
union U {
    wg: core::mem::ManuallyDrop<WriteGuard>,
    none: bool,
}

#[repr(C)]
struct LockResult {
    tag: Tag,
    u: U,
}

#[link(name = "main", kind = "static")]
extern "C" {
    fn new_buffer() -> RingBuffer;

    fn lock_buffer(rb: *mut RingBuffer) -> LockResult;

    fn get_reader(rb: *mut RingBuffer) -> SharedReader;

    fn push_back(wg: *mut WriteGuard, val: u64);

    fn pop_front(sr: *mut SharedReader) -> u64;

    fn drop_wg(wg: *mut WriteGuard);
}

impl RingBuffer {
    /// Constructs a new, empty array with a fixed length.
    /// ```rust
    /// # use zsling::*;
    /// let buffer: RingBuffer = RingBuffer::new();
    /// ```
    pub fn new() -> Self {
        unsafe { new_buffer() }
    }

    /// Tries to acquire the [`RingBuffer's`] [`WriteGuard`]. As there can
    /// only ever be one thread holding a [`WriteGuard`], this fails if another thread is
    /// already holding the lock.
    /// ```rust
    /// # use zsling::*;
    /// let buffer: RingBuffer = RingBuffer::new();
    ///
    /// let Ok(mut writer) = buffer.try_lock() else { return };
    /// ```
    pub fn try_lock(&self) -> Result<WriteGuard, ()> {
        return unsafe {
            match lock_buffer(self as *const RingBuffer as *mut _) {
                LockResult {
                    tag: Tag::Success,
                    u,
                } => Ok(core::mem::ManuallyDrop::into_inner(u.wg)),
                _ => Err(()),
            }
        };
    }

    /// Creates a new [`SharedReader`] which provides shared read access of the queue. The
    /// progress of this [`SharedReader`] is not affected by other
    /// [`SharedReader`]s.
    /// and does not affect them in turn.
    /// ```rust
    /// # use zsling::*;
    /// let buffer: RingBuffer = RingBuffer::new();
    ///
    /// let reader = buffer.reader();
    /// ```
    pub fn reader(&self) -> SharedReader {
        unsafe { get_reader(self as *const RingBuffer as *mut _) }
    }
}

unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl WriteGuard {
    /// Push a new value to the back of the queue. This operation does not block.
    /// ```rust
    /// # use zsling::*;
    /// let buffer: RingBuffer = RingBuffer::new();
    ///
    /// if let Ok(mut writer) = buffer.try_lock() {
    ///     writer.push_back([0, 1, 2, 3, 4, 5, 6, 7])
    /// };
    /// ```
    pub fn push_back(&mut self, val: [u8; 8]) {
        unsafe {
            push_back(self, core::mem::transmute(val));
        }
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        unsafe { drop_wg(self) }
    }
}

unsafe impl Send for WriteGuard {}
unsafe impl Sync for WriteGuard {}

impl SharedReader {
    /// Pops the next element from the front. The element is only popped for us and other threads
    /// will still need to pop this for themselves.
    /// ```rust
    /// # use zsling::*;
    /// # let do_something = |data| {};
    /// let buffer: RingBuffer = RingBuffer::new();
    ///
    /// let reader = buffer.reader();
    ///
    /// if let Some(data) = reader.pop_front() {
    ///    do_something(data);
    /// };
    /// ```
    pub fn pop_front(&self) -> Option<[u8; 8]> {
        unsafe {
            let res = pop_front(self as *const SharedReader as *mut SharedReader);
            match res {
                u64::MAX => None,
                x => Some(core::mem::transmute(x)),
            }
        }
    }
}

unsafe impl Send for SharedReader {}
unsafe impl Sync for SharedReader {}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::println;

    #[test]
    fn it_works() {
        let buffer = RingBuffer::new();
        let mut writer = buffer.try_lock().unwrap();
        assert!(buffer.try_lock().is_err());
        let reader = buffer.reader();
        writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
        println!("{:?}", reader.pop_front().unwrap());

        writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 7], reader.pop_front().unwrap());

        writer.push_back([0, 1, 2, 3, 4, 5, 6, 14]);
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 14], reader.pop_front().unwrap());

        println!("{:?}", reader.pop_front());

        drop(writer);

        assert!(buffer.try_lock().is_ok());
    }
}
