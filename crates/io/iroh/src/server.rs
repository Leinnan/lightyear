use aeronet_io::connection::PeerAddr;
use aeronet_iroh::SessionEstablished;
use aeronet_iroh::iroh::Endpoint;
use aeronet_iroh::server::{IrohServer, IrohServerClient};
pub use aeronet_iroh::server::{SessionRequest, SessionResponse};
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_aeronet::endpoint::EndpointAeronetPlugin;
use lightyear_aeronet::{AeronetLinkOf, AeronetPlugin};
use lightyear_link::prelude::LinkOf;
use lightyear_link::server::Server;
use lightyear_link::{Link, LinkStart, Linked, Linking};
use tracing::info;

/// Allows using [`IrohServerIo`] as an IO layer.
pub struct IrohServerPlugin;

/// iroh QUIC server IO component.
///
/// Attach this to an entity to start an iroh server when [`LinkStart`] is triggered.
/// The entity automatically gains a [`Server`] component (required).
///
/// Accepted client sessions fire [`SessionRequest`] on the server entity —
/// the application **must** respond with [`SessionRequest::respond`] using
/// [`SessionResponse::Accepted`] or [`SessionResponse::Rejected`].
///
/// The [`aeronet_iroh::IrohRuntime`] resource must be present (auto-inserted
/// by [`IrohServerPlugin`] if absent).
#[derive(Component)]
#[require(Server)]
pub struct IrohServerIo {
    /// The already-bound iroh endpoint to listen on.
    pub endpoint: Endpoint,
}

impl Plugin for IrohServerPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AeronetPlugin>() {
            app.add_plugins(AeronetPlugin);
        }
        if !app.is_plugin_added::<EndpointAeronetPlugin>() {
            app.add_plugins(EndpointAeronetPlugin);
        }
        app.add_plugins(aeronet_iroh::server::IrohServerPlugin);
        app.add_observer(Self::link);
        app.add_observer(Self::on_connection);
    }
}

impl IrohServerPlugin {
    fn link(
        trigger: On<LinkStart>,
        query: Query<(Entity, &IrohServerIo), (Without<Linking>, Without<Linked>)>,
        mut commands: Commands,
    ) -> Result {
        if let Ok((entity, io)) = query.get(trigger.entity) {
            let endpoint = io.endpoint.clone();
            commands.queue(move |world: &mut World| {
                info!("IrohServer starting");
                let child = world.spawn((AeronetLinkOf(entity), Name::from("IrohServer")));
                IrohServer::open(endpoint).apply(child);
            });
        }
        Ok(())
    }

    /// Observes [`SessionEstablished`] (fired by `aeronet_iroh` when the QUIC
    /// handshake completes) to create a lightyear [`LinkOf`] entity for each
    /// accepted client session.
    ///
    /// # Why `SessionEstablished` instead of `On<Add, Session>`?
    ///
    /// The previous implementation observed `On<Add, Session>` and queried
    /// `(&ChildOf, &PeerAddr)` — a **required** `&PeerAddr` that is absent for
    /// relay-only connections (where no direct socket path was selected).
    /// This caused relay sessions to be silently dropped: the query returned
    /// `Err`, the `if let Ok(...)` swallowed the failure, and no `LinkOf`
    /// entity was ever created.
    ///
    /// [`SessionEstablished`] carries `peer_addr: Option<SocketAddr>` as a
    /// field, so this observer handles both direct and relay-only sessions
    /// correctly without any fragile component queries.
    fn on_connection(
        trigger: On<SessionEstablished>,
        aeronet_query: Query<&AeronetLinkOf>,
        child_query: Query<&ChildOf, With<IrohServerClient>>,
        mut commands: Commands,
    ) {
        let event = trigger.event();
        let session_entity = event.entity;

        let Ok(child_of) = child_query.get(session_entity) else {
            return;
        };
        let Ok(server_link) = aeronet_query.get(child_of.parent()) else {
            return;
        };

        // Spawn the lightyear LinkOf entity that represents this client
        // connection in the higher-level connection/transport/messages layers.
        let mut link_spawn = commands.spawn((
            LinkOf {
                endpoint: server_link.0,
            },
            Link::default(),
        ));

        // PeerAddr is optional: relay-only connections have no direct socket
        // address. Insert it only when available so downstream components that
        // observe `On<Add, PeerAddr>` receive accurate data.
        if let Some(peer_addr) = event.peer_addr {
            link_spawn.insert(PeerAddr(peer_addr));
        }

        let link_entity = link_spawn.id();

        // Connect the aeronet session entity → lightyear link entity so that
        // `lightyear_aeronet`'s receive/send systems can bridge packet buffers.
        commands
            .entity(session_entity)
            .insert((AeronetLinkOf(link_entity), Name::from("IrohClientOf")));
    }
}
