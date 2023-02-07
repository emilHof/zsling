#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use zsling::RingBuffer;

const VALID: [u8; 32] = [
    00, 01, 02, 03, 04, 05, 06, 07, 08, 09, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
    24, 25, 26, 27, 28, 29, 30, 31,
];

#[derive(Debug, Clone, Copy, Arbitrary)]
enum Message {
    A,
    B,
    C,
    D,
}

fn range(m: Message) -> [u8; 8] {
    let mut empty = [0; 8];
    match m {
        Message::A => empty.copy_from_slice(&VALID[..8]),
        Message::B => empty.copy_from_slice(&VALID[8..16]),
        Message::C => empty.copy_from_slice(&VALID[16..24]),
        Message::D => empty.copy_from_slice(&VALID[24..]),
    }
    empty
}

const ALL: [Message; 4] = [Message::A, Message::B, Message::C, Message::D];

const MAX_SPIN: usize = 64;
const BUFF_SIZE: usize = u8::MAX as usize;

fuzz_target!(|data: Vec<Message>| {
    // fuzzed code goes here
    let buffer = RingBuffer::new();
    let mut writer = buffer.try_lock().unwrap();
    let mut reader = buffer.reader();
    let mut valid = std::collections::HashSet::new();
    ALL.iter().for_each(|m| {
        valid.insert(range(m.clone()));
    });

    std::thread::scope(|s| {
        let reader = &reader;
        let valid = &valid;

        for _ in 0..8 {
            s.spawn(move || loop {
                while let Some(m) = reader.pop_front() {
                    assert!(valid.contains(&m));
                }

                let mut counter = 0;
                while reader.pop_front().is_none() && counter < MAX_SPIN {
                    counter += 1;
                    std::thread::yield_now();
                }

                if counter < MAX_SPIN {
                    continue;
                }

                break;
            });
        }

        for window in data.windows(8) {
            for &message in window {
                writer.push_back(range(message));
            }
            std::thread::yield_now();
        }
    })
});
