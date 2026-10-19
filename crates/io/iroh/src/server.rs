use aeronet_io::Session;
use aeronet_iroh::endpoint::{EndpointClosed, IrohEndpoint};
use aeronet_iroh::iroh::endpoint::Builder;
use aeronet_iroh::session::{IrohSession, SessionSide};
pub use aeronet_iroh::session::{SessionRequest, SessionResponse};
use alloc::format;
use alloc::sync::Arc;
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_aeronet::{AeronetLinkOf, AeronetPlugin};
use lightyear_link::endpoint::EndpointLinkPlugin;
use lightyear_link::prelude::LinkOf;
use lightyear_link::server::Server;
use lightyear_link::{Link, LinkStart, Linked, Linking, UnlinkReason, Unlinked};
use tracing::info;

use crate::endpoint::{self, BuilderFn, IrohLinkEndpoint, IrohRemoteId};

/// Allows using [`IrohServerIo`] as an IO layer.
pub struct IrohServerPlugin;

/// iroh QUIC server IO component.
///
/// Attach this to an entity to start an iroh server when [`LinkStart`] is triggered.
/// The entity automatically gains a [`Server`] component (required).
///
/// On [`LinkStart`] a child [`IrohEndpoint`] entity is opened from a fresh [`Builder`]; its entity
/// is stored in [`IrohLinkEndpoint`]. The builder must accept the ALPN clients connect with.
///
/// Every incoming session fires [`SessionRequest`] on that endpoint entity — the application
/// **must** respond with [`SessionRequest::respond`] using [`SessionResponse::Accepted`] or
/// [`SessionResponse::Rejected`].
///
/// The [`aeronet_iroh::IrohRuntime`] resource is inserted automatically if absent.
#[derive(Component)]
#[require(Server)]
pub struct IrohServerIo {
    /// Creates the iroh endpoint builder each time the server starts.
    pub builder: BuilderFn,
    /// Open the endpoint with gossip room discovery. The builder must also accept
    /// `iroh_gossip::net::GOSSIP_ALPN`.
    #[cfg(feature = "discovery")]
    pub discovery: bool,
}

impl IrohServerIo {
    /// Create a server IO whose endpoint is built by `builder` on every [`LinkStart`].
    pub fn new(builder: impl Fn() -> Builder + Send + Sync + 'static) -> Self {
        Self {
            builder: Arc::new(builder),
            #[cfg(feature = "discovery")]
            discovery: false,
        }
    }
}

impl Plugin for IrohServerPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AeronetPlugin>() {
            app.add_plugins(AeronetPlugin);
        }
        if !app.is_plugin_added::<EndpointLinkPlugin>() {
            app.add_plugins(EndpointLinkPlugin);
        }
        endpoint::add_iroh_plugin(app);
        app.add_observer(Self::link);
        app.add_observer(Self::on_opened);
        app.add_observer(Self::on_closed);
        app.add_observer(Self::on_unlinked);
        app.add_observer(Self::on_connection);
    }
}

impl IrohServerPlugin {
    fn link(
        trigger: On<LinkStart>,
        query: Query<&IrohServerIo, (Without<Linking>, Without<Linked>)>,
        mut commands: Commands,
    ) -> Result {
        if let Ok(io) = query.get(trigger.entity) {
            info!("IrohServer starting");
            #[cfg(feature = "discovery")]
            let discovery = io.discovery;
            #[cfg(not(feature = "discovery"))]
            let discovery = false;
            let endpoint = commands
                .spawn((AeronetLinkOf(trigger.entity), Name::from("IrohServer")))
                .queue(endpoint::open((io.builder)(), discovery))
                .id();
            commands
                .entity(trigger.entity)
                .insert((Linking, IrohLinkEndpoint(endpoint)));
        }
        Ok(())
    }

    /// The endpoint has bound: the server is linked.
    fn on_opened(
        trigger: On<Add, IrohEndpoint>,
        endpoint_query: Query<&AeronetLinkOf>,
        server_query: Query<(), With<IrohServerIo>>,
        mut commands: Commands,
    ) {
        if let Ok(server) = endpoint_query.get(trigger.entity)
            && server_query.contains(server.0)
        {
            commands.entity(server.0).insert(Linked);
        }
    }

    /// The endpoint failed to bind or closed unexpectedly.
    fn on_closed(
        trigger: On<EndpointClosed>,
        endpoint_query: Query<&AeronetLinkOf>,
        server_query: Query<(), With<IrohServerIo>>,
        mut commands: Commands,
    ) {
        if let Ok(server) = endpoint_query.get(trigger.entity)
            && server_query.contains(server.0)
        {
            commands.entity(server.0).try_insert(Unlinked {
                reason: UnlinkReason::TransportError(format!("{}", trigger.error)),
            });
        }
    }

    /// Close the endpoint whenever the server becomes unlinked. `EndpointLinkPlugin` already
    /// unlinks and despawns the server's [`LinkOf`] entities.
    fn on_unlinked(
        trigger: On<Add, Unlinked>,
        query: Query<&IrohLinkEndpoint, With<IrohServerIo>>,
        mut commands: Commands,
    ) {
        if let Ok(endpoint) = query.get(trigger.entity) {
            commands.entity(endpoint.0).try_despawn();
            commands.entity(trigger.entity).remove::<IrohLinkEndpoint>();
        }
    }

    /// Creates a lightyear [`LinkOf`] entity for each accepted incoming session, once its
    /// [`Session`] is established.
    ///
    /// `PeerAddr` is not copied here: `lightyear_aeronet` propagates it once [`AeronetLinkOf`] is
    /// inserted, and relay-only sessions simply have none.
    fn on_connection(
        trigger: On<Add, Session>,
        session_query: Query<&IrohSession>,
        endpoint_query: Query<&AeronetLinkOf>,
        server_query: Query<(), With<IrohServerIo>>,
        mut commands: Commands,
    ) {
        let session_entity = trigger.entity;
        let Ok(session) = session_query.get(session_entity) else {
            return;
        };
        if session.side() != SessionSide::Incoming {
            return;
        }
        let Ok(server) = endpoint_query.get(session.endpoint()) else {
            return;
        };
        if !server_query.contains(server.0) {
            return;
        }

        // Spawn the lightyear LinkOf entity that represents this client
        // connection in the higher-level connection/transport/messages layers.
        let link_entity = commands
            .spawn((
                LinkOf { endpoint: server.0 },
                Link::default(),
                IrohRemoteId(session.peer_id()),
            ))
            .id();

        // Connect the aeronet session entity → lightyear link entity so that
        // `lightyear_aeronet`'s receive/send systems can bridge packet buffers.
        commands
            .entity(session_entity)
            .insert((AeronetLinkOf(link_entity), Name::from("IrohClientOf")));
    }
}
