//! The Engine.IO v4 server. Unstable and hidden until the Socket.IO server builds on it.

pub mod options;
pub mod packet;
pub mod sid;

#[cfg(test)]
mod fuzz;
