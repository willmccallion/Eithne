//! Page-level constants and order helpers.

/// Standard page size (4 KiB).
pub const PAGE_SIZE: usize = 4096;

/// Large page size (2 MiB on x86_64, varies on other archs).
pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

/// Number of buddy allocator orders (0..=MAX_ORDER-1).
/// Order 0 = 4 KiB, Order 10 = 4 MiB. Matches Linux.
pub const MAX_ORDER: usize = 11;

/// Convert a buddy order to its page count (2^order).
#[allow(clippy::inline_always)]
#[inline(always)]
pub const fn order_to_pages(order: usize) -> usize {
    1 << order
}

/// Convert a buddy order to its byte size.
#[allow(clippy::inline_always)]
#[inline(always)]
pub const fn order_to_bytes(order: usize) -> usize {
    PAGE_SIZE << order
}

/// Find the smallest order whose block size fits `pages` pages.
/// E.g. 1 page → order 0, 3 pages → order 2 (4 pages).
#[inline]
pub const fn pages_to_order(pages: usize) -> usize {
    if pages <= 1 {
        return 0;
    }
    // Round up to next power of 2, then trailing_zeros gives the order
    let rounded = pages.next_power_of_two();
    rounded.trailing_zeros() as usize
}
