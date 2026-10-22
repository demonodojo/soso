//! Pipeline distribuido en estrella (head + nodos), genérico sobre [`crate::pipeline::Transport`].

mod estrella;
mod nodo;

pub use estrella::{Conector, EstrellaConfig, EstrellaHead, EnlaceRemoto, HandshakeCaps};
pub use estrella::PendingEnlace;
pub use nodo::{NodoConfig, NodoPipeline};
