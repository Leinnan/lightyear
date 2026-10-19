extern crate alloc;

#[cfg(feature = "client")]
pub mod client;
pub mod connection;
pub mod endpoint;
#[cfg(feature = "server")]
pub mod server;

#[derive(thiserror::Error, Debug)]
pub enum IrohError {
    #[error("IrohRuntime resource missing; insert IrohRuntime before starting")]
    RuntimeMissing,
}

pub use connection::{IrohConnectionPlugin, endpoint_id_to_peer_id};
pub use endpoint::{BuilderFn, IrohLinkEndpoint, IrohRemoteId};

/// Gossip-based room discovery using iroh-gossip.
///
/// Set `discovery: true` on the IO component (and include
/// `iroh_gossip::net::GOSSIP_ALPN` in the builder's ALPNs) so its
/// endpoint gets a [`GossipHandle`]. Then start [`AnnounceRoom`] or [`SearchRooms`] with the
/// endpoint entity from [`IrohLinkEndpoint`](crate::IrohLinkEndpoint). Discovered rooms are stored
/// in the [`DiscoveredRooms`] resource and surfaced via [`RoomDiscovered`] / [`RoomLost`] events.
#[cfg(feature = "discovery")]
pub mod discovery {
    pub use aeronet_iroh::discovery::{
        AnnounceRoom, DiscoveredRooms, DiscoveryConfig, GossipHandle, IrohDiscoveryPlugin,
        RoomAnnouncement, RoomDiscovered, RoomInfo, RoomLost, SearchRooms,
    };
}

pub mod prelude {
    pub use crate::IrohError;
    pub use crate::connection::IrohConnectionPlugin;
    pub use crate::endpoint::{IrohLinkEndpoint, IrohRemoteId};
    pub use aeronet_iroh::IrohRuntime;
    pub use aeronet_iroh::endpoint::{EndpointClosed, IrohEndpoint};

    #[cfg(feature = "client")]
    pub mod client {
        pub use crate::client::{IrohClientIo, IrohClientPlugin, IrohTarget};
    }

    #[cfg(feature = "server")]
    pub mod server {
        pub use crate::server::{IrohServerIo, IrohServerPlugin};
        pub use aeronet_iroh::session::{SessionRequest, SessionResponse};
    }

    #[cfg(feature = "discovery")]
    pub mod discovery {
        pub use crate::discovery::*;
    }
}
