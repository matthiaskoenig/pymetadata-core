//! The panic hook of this host, which leaves a sentence behind before the trap.
//!
//! `wasm32-unknown-unknown` unwinds nothing: a panic runs the hook and then traps, and a page
//! catches a `RuntimeError` with no message of its own. The hook is what turns that into a
//! sentence, by publishing it as the error output of kind `panic` which the loader reads after
//! it caught the trap. No input of a page is supposed to reach it: malformed bytes are an
//! `OmexError` throughout the core, so a panic here is a bug of this build.
//!
//! The hook is as dumb as it can be, because a panic inside a panic hook aborts the instance
//! without any message at all: it formats the message, serializes two fields and assigns one
//! `Vec`. None of the three can panic. An allocation which the instance cannot serve aborts, it
//! does not panic, so the hook cannot re-enter itself through one either. It also publishes
//! into a static of its own kind: the call it interrupts holds a borrow of the archive or a
//! slice of the input buffer, and the output buffer is neither (see [`crate::buffers`]).

use std::sync::Once;

use crate::buffers;
use crate::calls::{PANIC, response};

/// Install the hook, once per instance.
///
/// Every export calls this first, so a page gets the sentence whichever export it started with.
/// The hook which was there before is kept and called after, so a panic of a native test of
/// this crate still prints what it would have printed.
pub(crate) fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            buffers::publish(response(PANIC, info.to_string()));
            previous(info);
        }));
    });
}
