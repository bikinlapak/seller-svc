//! PasStok's deterministic core: forecast, reorder, expiry risk, waste
//! destinations and the daily special. Pure functions over plain data, so
//! every rule is unit-tested without a server, a database or a model.
//!
//! Design principle (PRD §8): numbers come from here, words come from the
//! agent. The language model never computes a quantity.

pub mod expiry;
pub mod forecast;
pub mod reorder;
pub mod special;
pub mod waste;
