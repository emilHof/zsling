const std = @import("std");
const AtomicOrder = std.atomic.Ordering;
const testing = std.testing;

fn Block(comptime S: u64) type {
    return struct {
        version: u64,
        message: [S]u8,
    };
}

pub fn RingBuffer(comptime S: u64, comptime N: u64) type {
    return struct {
        const Self = @This();

        index: u64,
        version: u64,
        locked: bool,
        data: [N]Block(S),

        pub fn init() Self {
            return Self {
                .index = 0,
                .version = 0,
                .locked = false,
                .data = [N]Block(S),
            };
        }

        pub const WriteGuard = struct {
            buffer: *RingBuffer(S, N),
        };

        pub fn lock(self: *Self) !WriteGuard {
            if @cmpxchg(&self.locked, false, true, Ordering.Unordered, Ordering.Unordered) {

            }
            _ = self;

        }
    };
}

test "basic add functionality" {
    try testing.expect(add(3, 7) == 10);
}
