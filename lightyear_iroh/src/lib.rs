#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

#[derive(thiserror::Error, Debug)]
pub enum IrohError {
    #[error("IrohRuntime resource missing; insert IrohRuntime before starting")]
    RuntimeMissing,
}

/// Gossip-based room discovery using iroh-gossip.
///
/// Servers call [`AnnounceRoom`] to broadcast their presence; clients call
/// [`SearchRooms`] to find available games. Discovered rooms are stored in
/// the [`DiscoveredRooms`] resource and surfaced via [`RoomDiscovered`] /
/// [`RoomLost`] events.
#[cfg(feature = "discovery")]
pub mod discovery {
    pub use aeronet_iroh::{
        AnnounceRoom, DiscoveredRooms, DiscoveryConfig, GossipHandle, IrohDiscoveryPlugin,
        RoomAnnouncement, RoomDiscovered, RoomInfo, RoomLost, SearchRooms,
    };
}

pub mod prelude {
    pub use crate::IrohError;
    pub use aeronet_iroh::IrohRuntime;

    #[cfg(feature = "client")]
    pub mod client {
        pub use crate::client::{IrohClientIo, IrohClientPlugin, IrohTarget};
    }

    #[cfg(feature = "server")]
    pub mod server {
        pub use crate::server::{IrohServerIo, IrohServerPlugin};
        pub use aeronet_iroh::server::{SessionRequest, SessionResponse};
    }

    #[cfg(feature = "discovery")]
    pub mod discovery {
        pub use crate::discovery::*;
    }
}
