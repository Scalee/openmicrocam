//! The camera's menu program: what the remote's keys open and change on the screen, drawn by microcamd as a scene and
//! asked of it over its control socket.
//!
//! - [`core`] — the menu without I/O: a key, an event or a reply, or the clock → the state and the requests.
//! - [`menu`] — the tables: sections, items, what each shows and does.
//! - [`text`] — the words, Russian and English.
//! - [`view`] — the state as the whole scene tree and the pictures it names.
//! - [`presenter`] — the tree onto microcamd's outputs, paced by what they show.

pub mod core;
pub mod menu;
pub mod presenter;
pub mod text;
pub mod view;
