#![forbid(unsafe_code)]

//! Host-agnostic pages.
//!
//! Most pages belong to exactly one surface and live in [`crate::hosts`]. These
//! two do not: the health page is the same on every host, and the home page is a
//! thin alias kept so the original module path still resolves after the surface
//! split.

pub mod health;
pub mod home;
