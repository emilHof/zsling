//! This crates provides a sequentially locking Ring Buffer. It allows for
//! a fast and non-writer-blocking SPMC-queue, where all consumers read all
//! messages.
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
//! # Important!
//!
//! It is also important to keep in mind, that slow readers will be overrun by the writer if they
//! do not consume messages quickly enough. This can happen quite frequently if the buffer size is
//! not large enough. It is advisable to test applications on a case-by-case basis and find a
//! buffer size that is optimal to your use-case.
//! Error value returned by a failed write lock.

const std = @import("std");
const builtin = @import("builtin");
// const bench = @import("root").bench;
const bench = @import("./bench.zig");
const Ordering = std.atomic.Ordering;
const testing = std.testing;
const print = std.log.warn;

pub const BufferErrors = error{LockError};

fn Block(comptime S: usize) type {
    return extern struct {
        version: usize,
        message: [S]u8,
    };
}

fn Padded(comptime T: type) type {
    return switch (builtin.target.cpu.arch) {
        .x86_64, .sparc64, .aarch64, .powerpc64 => extern struct {
            data: T align(16),
        },
        .arm, .mips, .mips64, .riscv64 => extern struct { data: T align(4) },
        else => extern struct { data: T align(8) },
    };
}

/// A fixed-size, non-write-blocking, ring buffer, that behaves like a
/// SPMC queue and can be safely shared across threads.
/// It is limited to only work for types that are copy, as multiple
/// threads can read the same message.
pub fn RingBuffer(comptime S: usize, comptime N: usize) type {
    return extern struct {
        const Self = @This();

        // what else goes here?
        // version?
        // TODO(Emil): Can we make sure this is properly aligned for cache loads?
        index: Padded(usize),
        version: Padded(usize),
        locked: Padded(bool),
        data: [N]Block(S),

        /// Constructs a new, empty [`RingBuffer`] with a fixed length.
        pub fn new() Self {
            var self = Self{
                .index = std.mem.zeroes(Padded(usize)),
                .version = std.mem.zeroes(Padded(usize)),
                .locked = std.mem.zeroes(Padded(bool)),
                .data = std.mem.zeroes([N]Block(S)),
            };

            return self;
        }

        /// Increments the sequence at the current index by 1, making it odd, prohibiting reads.
        fn start_write(self: *Self) usize {
            var index: usize = self.index.data;
            var seq = self.data[index].version;
            @atomicStore(usize, &self.data[index].version, seq + 1, Ordering.Unordered);

            std.debug.assert(seq & 1 == 0);

            @atomicStore(usize, &self.version.data, seq + 2, Ordering.Unordered);

            return index;
        }

        /// Increments the sequence at the current index by 1, making it even and allowing reads.
        fn end_write(self: *Self, index: usize) void {
            @atomicStore(usize, &self.index.data, (index + 1) % N, Ordering.Unordered);
            var seq = self.data[index].version;
            @atomicStore(usize, &self.data[index].version, seq + 1, Ordering.Release);

            std.debug.assert(seq & 1 == 1);
        }

        /// Provides exclusive write access to the [`RingBuffer`].
        pub const WriteGuard = extern struct {
            buffer: *RingBuffer(S, N),

            /// Push a new value to the back of the queue. This operation does not block.
            pub fn push_back(wg: *WriteGuard, val: [S]u8) void {
                var i: usize = wg.buffer.start_write();
                @memcpy(&wg.buffer.data[i].message, &val, @sizeOf([S]u8));
                wg.buffer.end_write(i);
            }

            pub fn drop(wg: WriteGuard) void {
                @atomicStore(bool, &wg.buffer.locked.data, false, Ordering.Release);
            }
        };

        /// Tries to acquire the [`RingBuffer's`] [`WriteGuard`]. As there can
        /// only ever be one thread holding a [`WriteGuard`], this fails if another thread is
        /// already holding the lock.
        pub fn lock(self: *Self) !WriteGuard {
            if (@cmpxchgStrong(bool, &self.locked.data, false, true, Ordering.Acquire, Ordering.Monotonic) == null) {
                return WriteGuard{
                    .buffer = self,
                };
            }

            return BufferErrors.LockError;
        }

        /// Shared read access to its buffer. When multiple threads consume from the
        /// [`RingBuffer`] throught the same [`SharedReader`], they will share progress
        /// on the queue. Distinct [`RingBuffers`] do not share progress.
        pub const SharedReader = extern struct {
            buffer: *RingBuffer(S, N),
            index: Padded(usize),
            version: Padded(usize),

            /// Pops the next element from the front. The element is only popped for us and other threads
            /// will still need to pop this for themselves.
            pub fn pop_front(sr: *SharedReader) ?[S]u8 {
                var i: usize = @atomicLoad(usize, &sr.index.data, Ordering.Acquire);

                while (true) {
                    var ver: usize = @atomicLoad(usize, &sr.version.data, Ordering.Unordered);

                    // Ensures we are not reading old data, or data that is currently being written to.
                    // This is `Acquire` so we observed the write to data should seq1 == seq2.
                    var seq1: usize = @atomicLoad(usize, &sr.buffer.data[i].version, Ordering.Acquire);

                    if (!check_seq(seq1, ver, i)) {
                        return null;
                    }

                    var data: [S]u8 = undefined;

                    // We cannot test the this part of the process with `loom`, as this operation is `UB`
                    // if data is written too while we are reading it; yet, due to the nature of seqlock,
                    // we discard the `UB` reads. Future versions of the compiler may optimize this code in
                    // a way that allows `UB` reads to leak past the seqlock, but currently this
                    // implementation is sane.
                    @memcpy(&data, &sr.buffer.data[i].message, @sizeOf([S]u8));

                    var seq2: usize = @atomicLoad(usize, &sr.buffer.data[i].version, Ordering.Acquire);

                    if (seq1 != seq2) {
                        continue;
                    }

                    // On failure we end here, as we have an outdated version and thus are reading consumed
                    // data.
                    if (@cmpxchgStrong(usize, &sr.version.data, ver, seq2, Ordering.Monotonic, Ordering.Monotonic) != null) {
                        return null;
                    }

                    // If this fails, someone has already read the data. This is the only time we should
                    // retry the loop.
                    // This is `Release` on store to ensure that the new version of the `SharedReader` is
                    // observed by all sharing threads, and on failure we `Acquire` to ensure we get the
                    // latest version.
                    if (@cmpxchgStrong(usize, &sr.index.data, i, (i + 1) % N, Ordering.Release, Ordering.Acquire)) |*now| {
                        i = now.*;
                        continue;
                    }

                    return data;
                }
            }

            /// Checks if we are reading data we have already consumed.
            fn check_seq(seq: usize, ver: usize, i: usize) bool {
                if (seq & 1 != 0) {
                    return false;
                }

                if ((i == 0 and seq == ver) or seq < ver) {
                    return false;
                }

                return true;
            }
        };

        /// Creates a new [`SharedReader`] which provides shared read access of the queue. The
        /// progress of this [`SharedReader`] is not affected by other
        /// [`SharedReader`]s.
        /// and does not affect them in turn.
        pub fn reader(self: *Self) SharedReader {
            return SharedReader{
                .buffer = self,
                .index = std.mem.zeroes(Padded(usize)),
                .version = std.mem.zeroes(Padded(usize)),
            };
        }
    };
}

const RB = RingBuffer(8, 256);
const WG = RB.WriteGuard;
const SR = RB.SharedReader;
const Tag = enum(c_int) { Success, Error };
const U = extern union { wg: WG, none: bool };
const LockResult = extern struct { t: Tag, u: U };

// C bindings
export fn new_buffer() callconv(.C) RB {
    return RB.new();
}

export fn lock_buffer(rb: *RB) callconv(.C) LockResult {
    var wg = rb.lock() catch return LockResult{ .t = .Error, .u = U{ .none = false } };
    return LockResult{ .t = .Success, .u = U{ .wg = wg } };
}

export fn get_reader(rb: *RB) callconv(.C) RB.SharedReader {
    return rb.reader();
}

export fn push_back(wg: *WG, val: u64) void {
    wg.push_back(@bitCast([8]u8, val));
}

export fn pop_front(sr: *SR) u64 {
    if (sr.pop_front()) |val| return @bitCast(u64, val);
    return std.math.maxInt(u64);
}

export fn drop_wg(wg: *WG) void {
    wg.drop();
}

const log_level: std.log.level = .info;
const MAX_SPIN: usize = 128;

test "test buffer" {
    var buffer = RB.new();

    {
        var result = lock_buffer(&buffer);
        var writer = switch (result.t) {
            .Error => return error.LockError,
            .Success => result.u.wg,
        };

        defer writer.drop();
        writer.push_back([_]u8{ 0, 1, 2, 3, 4, 5, 6, 7 });

        // we should not be able to acquire another reader while this one is alive.
        try testing.expectError(BufferErrors.LockError, buffer.lock());
    }

    var reader = buffer.reader();

    var writer = try buffer.lock();
    defer writer.drop();
    writer.push_back([_]u8{ 0, 1, 2, 3, 4, 5, 6, 7 });
    std.log.warn("val: {any}", .{pop_front(&reader)});
}

fn test_read_buffer(reader: *RB.SharedReader) void {
    var i: usize = 0;
    while (i < 100) : (i += 1) {
        std.Thread.yield() catch {};
        while (reader.pop_front()) |_| {
            std.Thread.yield() catch {};
        }

        var counter: usize = 0;

        while (reader.pop_front() == null and counter < MAX_SPIN) : (counter += 1) {
            std.Thread.yield() catch {};
        }

        if (counter < MAX_SPIN) {
            continue;
        }

        break;
    }
}

const Arr = std.ArrayList(std.Thread);
test "test with threads" {
    var buffer = RB.new();
    var writer = try buffer.lock();
    var reader = buffer.reader();

    var threads = Arr.init(testing.allocator);
    defer threads.deinit();
    errdefer threads.deinit();

    defer while (threads.popOrNull()) |thread| {
        thread.join();
    };

    var t: usize = try std.Thread.getCpuCount() + 1;
    while (t > 0) : (t -= 1) {
        try threads.append(try std.Thread.spawn(.{}, test_read_buffer, .{&reader}));
    }

    var i: usize = 0;
    while (i < 1_000) : (i += 1) {
        writer.push_back([_]u8{ 0, 1, 2, 3, 4, 5, 6, 7 });
        std.Thread.yield() catch {};
    }
}

fn test_ping(reader: *RB.SharedReader, buffer: *RB, pinged: *bool) !void {
    while (!@atomicLoad(bool, pinged, Ordering.Acquire)) {
        if (reader.pop_front() != null) {
            var w2: RB.WriteGuard = try buffer.lock();
            w2.push_back([_]u8{ 0, 1, 2, 3, 4, 5, 6, 7 });
            @atomicStore(bool, pinged, true, Ordering.Release);
        }
        try std.Thread.yield();
    }
}

test "bench" {
    try bench.benchmark(struct {
        pub const args = [_]usize{ 1, 2, 4, 8 };

        pub const arg_names = [_][]const u8{
            "threads=1",
            "threads=2",
            "threads=4",
            "threads=8",
        };

        pub const min_iterations = 100_000;
        pub const max_iterations = 500_000;

        pub fn ping(t: usize) !void {
            var b1 = comptime RB.new();
            var b2 = comptime RB.new();

            var w1 = try b1.lock();
            var r1 = b1.reader();
            var r2 = b2.reader();

            var pinged: bool = false;

            var threads = Arr.init(testing.allocator);
            defer threads.deinit();
            errdefer threads.deinit();

            defer while (threads.popOrNull()) |thread| {
                thread.join();
            };

            var i: usize = 0;

            while (i < t) : (i += 1) {
                try threads.append(try std.Thread.spawn(.{}, test_ping, .{ &r1, &b2, &pinged }));
            }

            w1.push_back([_]u8{ 0, 1, 2, 3, 4, 5, 6, 7 });
            while (r2.pop_front() == null) {
                std.Thread.yield() catch {};
            }
        }
    });
}
