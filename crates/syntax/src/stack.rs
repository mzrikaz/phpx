//! Stack budget for the recursive parts of the front end (parser, tree
//! walks). Deeply nested source would otherwise overflow the thread's
//! stack and abort the process; past the budget, parsing fails with PHP's
//! "Maximum call stack size ... reached during compilation" error instead.

use std::cell::Cell;

thread_local! {
    /// Lowest stack address recursion may reach (0: no budget set).
    static FLOOR: Cell<usize> = const { Cell::new(0) };
    /// The budget ran out since the last `take_exhausted`.
    static HIT: Cell<bool> = const { Cell::new(false) };
}

/// Budget used when the caller set none (the main thread's stack is
/// usually 8 MiB).
pub const DEFAULT_BUDGET: usize = 4 << 20;

#[inline(always)]
fn sp() -> usize {
    let marker = 0u8;
    std::ptr::addr_of!(marker) as usize
}

/// Run `f` with recursion limited to `bytes` of stack below this point
/// (or an existing, tighter limit).
pub fn with_budget<T>(bytes: usize, f: impl FnOnce() -> T) -> T {
    let old = FLOOR.with(Cell::get);
    let floor = sp().saturating_sub(bytes);
    FLOOR.with(|c| c.set(floor.max(old)));
    let r = f();
    FLOOR.with(|c| c.set(old));
    r
}

/// Like `with_budget` with the default budget, unless one is already set.
pub fn with_default_budget<T>(f: impl FnOnce() -> T) -> T {
    if FLOOR.with(Cell::get) != 0 {
        return f();
    }
    with_budget(DEFAULT_BUDGET, f)
}

/// Out of budget: callers stop recursing (and report it).
#[inline]
pub fn exhausted() -> bool {
    let floor = FLOOR.with(Cell::get);
    if floor != 0 && sp() < floor {
        HIT.with(|h| h.set(true));
        return true;
    }
    false
}

/// Whether the budget ran out since the last call (clears the flag).
pub fn take_exhausted() -> bool {
    HIT.with(|h| h.replace(false))
}

/// PHP's message for source nested too deeply to compile.
pub const TOO_DEEP: &str = "Maximum call stack size reached during compilation. Try splitting expression";
