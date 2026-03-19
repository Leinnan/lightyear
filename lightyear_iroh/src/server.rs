use aeronet_io::Session;
use aeronet_io::connection::PeerAddr;
use aeronet_iroh::iroh::Endpoint;
use aeronet_iroh::server::{IrohServer, IrohServerClient};
pub use aeronet_iroh::server::{SessionRequest, SessionResponse};
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_aeronet::server::ServerAeronetPlugin;
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
        if !app.is_plugin_added::<ServerAeronetPlugin>() {
            app.add_plugins(ServerAeronetPlugin);
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

    fn on_connection(
        trigger: On<Add, Session>,
        aeronet_query: Query<&AeronetLinkOf>,
        child_query: Query<(&ChildOf, &PeerAddr), With<IrohServerClient>>,
        mut commands: Commands,
    ) {
        if let Ok((child_of, peer_addr)) = child_query.get(trigger.entity)
            && let Ok(server_link) = aeronet_query.get(child_of.parent())
        {
            let link_entity = commands
                .spawn((
                    LinkOf {
                        server: server_link.0,
                    },
                    Link::new(None),
                    PeerAddr(peer_addr.0),
                ))
                .id();
            commands
                .entity(trigger.entity)
                .insert((AeronetLinkOf(link_entity), Name::from("IrohClientOf")));
        }
    }
}
