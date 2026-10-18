//! Concrete Skeleton animation-attribute consumers. These do not replace the
//! remaining ProcessInput and ProcessData pose/IK stages.
pub mod catalog;
pub mod name;
pub mod scalar_attributes;
pub mod contact_events;
pub mod extended_attributes;
pub mod attribute_finalization;
pub mod process_attributes;

#[cfg(test)]
mod tests;
