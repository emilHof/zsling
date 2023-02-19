This crates provides a sequentially locking Ring Buffer. It allows for
a fast and non-writer-blocking SPMC-queue, where all consumers read all
messages.

# Original

This is the Rust-wrapped Zig version of the [sling](https://crates.io/crates/sling) crate. Note
that due to current constraints in the implementation, the buffer size is set to 256 and the
and messages are set to [u8; 8].

# Usage

There are two ways of consuming from the queue. If threads share a
`SharedReader` through a shared reference, they will steal
queue items from one anothers such that no two threads will read the
same message. When a `SharedReader` is cloned, the new
`SharedReader`'s reading progress will no longer affect the other
one. If two threads each use a separate `SharedReader`, they
will be able to read the same messages.

```rust
# use zsling::*;

let buffer = RingBuffer::new();

let mut writer = buffer.try_lock().unwrap();
let mut reader = buffer.reader();

std::thread::scope(|s| {
    let reader = &reader;
    for t in 0..8 {
        s.spawn(move || {
            for _ in 0..100 {
                if let Some(val) = reader.pop_front() {
                    println!("t: {}, val: {:?}", t, val);
                };
            }
        });
    }

    for i in 0..100 {
        writer.push_back([0, 1, 2, 3, 4, 5, 6, 7]);
    }
});
```

# Important!

It is also important to keep in mind, that slow readers will be overrun by the writer if they
do not consume messages quickly enough. This can happen quite frequently if the buffer size is
not large enough. It is advisable to test applications on a case-by-case basis and find a
buffer size that is optimal to your use-case.
