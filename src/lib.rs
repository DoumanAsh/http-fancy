//!Fancy HTTP utilities aimed at `hyper`

#![no_std]
#![cfg_attr(feature = "cargo-clippy", allow(clippy::style))]
#![cfg_attr(rustfmt, rustfmt_skip)]
#![warn(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

macro_rules! unreach {
    () => {{
        #[cfg(not(debug_assertions))]
        unsafe {
            core::hint::unreachable_unchecked()
        }
        #[cfg(debug_assertions)]
        unreachable!()
    }}
}

pub mod body;
