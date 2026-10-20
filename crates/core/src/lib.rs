#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_div_ceil,
    clippy::needless_range_loop
)]
//! On-device catalog, search, schedules, and degree audit for Term Organize.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.
//! Course information is © Northeastern University. Portions derived from SearchNEU (AGPL-3.0).

pub mod archive;
pub mod banner;
pub mod calendar;
pub mod catalog;
pub mod course_info;
pub mod degree_audit;
pub mod degree_format;
pub mod editing;
pub mod email;
pub mod equivalences;
pub mod html;
pub mod index;
pub mod legal;
pub mod links;
pub mod model;
pub mod nl;
pub mod nupath;
pub mod offering;
pub mod outline;
pub mod petition;
pub mod planfile;
pub mod planlogic;
pub mod professors;
pub mod program_parser;
pub mod reorder;
pub mod reorganize;
pub mod rooms;
pub mod schedule;
pub mod search;
pub mod store;
pub mod suggestions;
pub mod sync;
pub mod titles;
pub mod upnext;
pub mod watchlist;

pub use equivalences::CourseEquivalences;
pub use model::*;
