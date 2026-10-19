//! Shared glue between lightyear link entities and `aeronet_iroh` endpoints.

use aeronet_iroh::IrohPlugin;
use aeronet_iroh::endpoint::IrohEndpoint;
use aeronet_iroh::iroh::EndpointId;
use aeronet_iroh::iroh::endpoint::Builder;
use alloc::sync::Arc;
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::system::EntityCommand;

/// Creates a fresh iroh endpoint [`Builder`] every time a link starts.
///
/// [`Builder`] is not `Clone`, and `aeronet_iroh` binds the endpoint itself, so the IO components
/// store a factory instead of a builder to support starting the same link more than once.
pub type BuilderFn = Arc<dyn Fn() -> Builder + Send + Sync>;

/// Points from a lightyear client or server entity to the [`IrohEndpoint`] entity opened for it.
///
/// Inserted on [`LinkStart`](lightyear_link::LinkStart). The endpoint component itself is only
/// present once the endpoint has bound. Use this entity to start
/// `aeronet_iroh` discovery (`AnnounceRoom` / `SearchRooms`) on the same endpoint.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrohLinkEndpoint(pub Entity);

/// Authenticated iroh identity of the remote peer of a server-side [`LinkOf`](lightyear_link::prelude::LinkOf).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IrohRemoteId(pub EndpointId);

pub(crate) fn add_iroh_plugin(app: &mut App) {
    if !app.is_plugin_added::<IrohPlugin>() {
        app.add_plugins(IrohPlugin);
    }
}

#[cfg_attr(not(feature = "discovery"), allow(unused_variables))]
pub(crate) fn open(builder: Builder, discovery: bool) -> impl EntityCommand {
    move |entity: EntityWorldMut| {
        #[cfg(feature = "discovery")]
        if discovery {
            IrohEndpoint::open_with_discovery(builder).apply(entity);
            return;
        }
        IrohEndpoint::open(builder).apply(entity);
    }
}
