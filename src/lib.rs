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

#[derive(Debug)]
#[repr(C)]
pub struct WriteGuard {
    buffer: *mut RingBuffer,
}

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
    wg: std::mem::ManuallyDrop<WriteGuard>,
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
    pub fn new() -> Self {
        unsafe { new_buffer() }
    }

    pub fn try_lock(&self) -> Result<WriteGuard, ()> {
        return unsafe {
            match lock_buffer(self as *const RingBuffer as *mut _) {
                LockResult {
                    tag: Tag::Success,
                    u,
                } => Ok(std::mem::ManuallyDrop::into_inner(u.wg)),
                _ => Err(()),
            }
        };
    }

    pub fn reader(&self) -> SharedReader {
        unsafe { get_reader(self as *const RingBuffer as *mut _) }
    }
}

unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl WriteGuard {
    pub fn push_back(&mut self, val: [u8; 8]) {
        unsafe {
            push_back(self, std::mem::transmute(val));
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
    pub fn pop_front(&self) -> Option<[u8; 8]> {
        unsafe {
            let res = pop_front(self as *const SharedReader as *mut SharedReader);
            match res {
                u64::MAX => None,
                x => Some(std::mem::transmute(x)),
            }
        }
    }
}

unsafe impl Send for SharedReader {}
unsafe impl Sync for SharedReader {}

#[cfg(test)]
mod tests {
    use super::*;

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
