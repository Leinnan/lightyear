use aeronet_iroh::client::IrohClient;
use aeronet_iroh::iroh::{Endpoint, EndpointAddr, EndpointId};
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_aeronet::{AeronetLinkOf, AeronetPlugin};
use lightyear_link::{Link, LinkStart, Linked, Linking};

/// Allows using [`IrohClientIo`] as an IO layer.
pub struct IrohClientPlugin;

/// Identifies the iroh server to connect to.
#[derive(Debug, Clone)]
pub enum IrohTarget {
    /// Connect using a full [`EndpointAddr`] (includes relay URL and direct socket addresses).
    Addr(EndpointAddr),
    /// Connect using just an [`EndpointId`] (the server's public key; iroh resolves routing via relay).
    Id(EndpointId),
}

impl From<IrohTarget> for EndpointAddr {
    fn from(target: IrohTarget) -> Self {
        match target {
            IrohTarget::Addr(addr) => addr,
            IrohTarget::Id(id) => EndpointAddr::from(id),
        }
    }
}

/// iroh QUIC client IO component.
///
/// Attach this to an entity alongside a [`Link`] (required automatically) to
/// connect to a remote iroh endpoint when [`LinkStart`] is triggered.
///
/// The [`aeronet_iroh::IrohRuntime`] resource must be present in the world
/// (it is inserted automatically by [`IrohClientPlugin`] if absent).
#[derive(Component)]
#[require(Link)]
pub struct IrohClientIo {
    /// The already-built local iroh endpoint.
    pub endpoint: Endpoint,
    /// The remote server to connect to.
    pub target: IrohTarget,
    /// Application-Level Protocol Negotiation bytes — must match the server's ALPN.
    pub alpn: Vec<u8>,
}

impl IrohClientIo {
    /// Create a new client IO targeting a full [`EndpointAddr`].
    pub fn from_addr(endpoint: Endpoint, addr: EndpointAddr, alpn: Vec<u8>) -> Self {
        Self {
            endpoint,
            target: IrohTarget::Addr(addr),
            alpn,
        }
    }

    /// Create a new client IO targeting an [`EndpointId`] (server's public key only).
    pub fn from_id(endpoint: Endpoint, id: EndpointId, alpn: Vec<u8>) -> Self {
        Self {
            endpoint,
            target: IrohTarget::Id(id),
            alpn,
        }
    }
}

impl Plugin for IrohClientPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AeronetPlugin>() {
            app.add_plugins(AeronetPlugin);
        }
        app.add_plugins(aeronet_iroh::client::IrohClientPlugin);
        app.add_observer(Self::link);
    }
}

impl IrohClientPlugin {
    fn link(
        trigger: On<LinkStart>,
        query: Query<(Entity, &IrohClientIo), (Without<Linking>, Without<Linked>)>,
        mut commands: Commands,
    ) -> Result {
        if let Ok((entity, io)) = query.get(trigger.entity) {
            let endpoint = io.endpoint.clone();
            let addr: EndpointAddr = io.target.clone().into();
            let alpn = io.alpn.clone();

            commands.queue(move |world: &mut World| -> Result {
                let entity_mut = world.spawn((AeronetLinkOf(entity), Name::from("IrohClient")));
                IrohClient::connect(endpoint, addr, alpn).apply(entity_mut);
                Ok(())
            });
        }
        Ok(())
    }
}
