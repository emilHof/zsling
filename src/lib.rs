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

#[link(name = "main", kind = "static")]
extern "C" {
    fn new_buffer() -> RingBuffer;

    fn lock_buffer(rb: *mut RingBuffer) -> WriteGuard;

    fn get_reader(rb: *mut RingBuffer) -> SharedReader;

    fn push_back(wg: *mut WriteGuard, val: u64);

    fn pop_front(sr: *mut SharedReader) -> u64;
}

impl RingBuffer {
    pub fn new() -> Self {
        unsafe { new_buffer() }
    }

    pub fn try_lock(&self) -> Result<WriteGuard, ()> {
        Ok(unsafe { lock_buffer(self as *const RingBuffer as *mut _) })
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
        let reader = buffer.reader();
        writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
        println!("{:?}", reader.pop_front().unwrap());

        writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 7], reader.pop_front().unwrap());

        writer.push_back([0, 1, 2, 3, 4, 5, 6, 14]);
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 14], reader.pop_front().unwrap());

        println!("{:?}", reader.pop_front());
    }
}
