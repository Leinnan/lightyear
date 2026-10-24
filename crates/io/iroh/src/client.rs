use aeronet_iroh::endpoint::{EndpointClosed, IrohEndpoint};
use aeronet_iroh::iroh::endpoint::Builder;
use aeronet_iroh::iroh::{EndpointAddr, EndpointId};
use aeronet_iroh::session::{SessionRequest, SessionResponse};
use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_aeronet::{AeronetLinkOf, AeronetPlugin};
use lightyear_link::{Link, LinkStart, Linked, Linking, UnlinkReason, Unlinked};

use crate::endpoint::{self, BuilderFn, IrohLinkEndpoint};

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
/// On [`LinkStart`] a local [`IrohEndpoint`] entity is opened from a fresh [`Builder`] (its entity
/// is stored in [`IrohLinkEndpoint`]); once bound, it connects to [`target`](Self::target).
/// The endpoint is closed when the client becomes [`Unlinked`].
///
/// The [`aeronet_iroh::IrohRuntime`] resource is inserted automatically if absent.
#[derive(Component)]
#[require(Link)]
pub struct IrohClientIo {
    /// Creates the local iroh endpoint builder each time the client starts.
    pub builder: BuilderFn,
    /// The remote server to connect to.
    pub target: IrohTarget,
    /// Application-Level Protocol Negotiation bytes — must match the server's ALPN.
    pub alpn: Vec<u8>,
    /// Open the endpoint with gossip room discovery. The builder must also accept
    /// `iroh_gossip::net::GOSSIP_ALPN`.
    #[cfg(feature = "discovery")]
    pub discovery: bool,
}

impl IrohClientIo {
    /// Create a new client IO targeting a full [`EndpointAddr`].
    pub fn from_addr(
        builder: impl Fn() -> Builder + Send + Sync + 'static,
        addr: EndpointAddr,
        alpn: Vec<u8>,
    ) -> Self {
        Self::new(builder, IrohTarget::Addr(addr), alpn)
    }

    /// Create a new client IO targeting an [`EndpointId`] (server's public key only).
    pub fn from_id(
        builder: impl Fn() -> Builder + Send + Sync + 'static,
        id: EndpointId,
        alpn: Vec<u8>,
    ) -> Self {
        Self::new(builder, IrohTarget::Id(id), alpn)
    }

    fn new(
        builder: impl Fn() -> Builder + Send + Sync + 'static,
        target: IrohTarget,
        alpn: Vec<u8>,
    ) -> Self {
        Self {
            builder: Arc::new(builder),
            target,
            alpn,
            #[cfg(feature = "discovery")]
            discovery: false,
        }
    }
}

/// Marks the [`IrohEndpoint`] entity opened for an [`IrohClientIo`] entity.
///
/// Not an [`AeronetLinkOf`]: that relationship belongs to the client's session entity.
#[derive(Component)]
struct IrohClientEndpointOf(Entity);

impl Plugin for IrohClientPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AeronetPlugin>() {
            app.add_plugins(AeronetPlugin);
        }
        endpoint::add_iroh_plugin(app);
        app.add_observer(Self::link);
        app.add_observer(Self::on_opened);
        app.add_observer(Self::on_closed);
        app.add_observer(Self::on_unlinked);
        app.add_observer(Self::reject_incoming);
    }
}

impl IrohClientPlugin {
    fn link(
        trigger: On<LinkStart>,
        query: Query<&IrohClientIo, (Without<Linking>, Without<Linked>)>,
        mut commands: Commands,
    ) -> Result {
        if let Ok(io) = query.get(trigger.entity) {
            #[cfg(feature = "discovery")]
            let discovery = io.discovery;
            #[cfg(not(feature = "discovery"))]
            let discovery = false;
            let endpoint = commands
                .spawn((
                    IrohClientEndpointOf(trigger.entity),
                    Name::from("IrohClientEndpoint"),
                ))
                .queue(endpoint::open((io.builder)(), discovery))
                .id();
            commands
                .entity(trigger.entity)
                .insert((Linking, IrohLinkEndpoint(endpoint)));
        }
        Ok(())
    }

    /// The local endpoint has bound: connect to the target.
    fn on_opened(
        trigger: On<Add<IrohEndpoint>>,
        endpoint_query: Query<(&IrohEndpoint, &IrohClientEndpointOf)>,
        client_query: Query<&IrohClientIo>,
        mut commands: Commands,
    ) {
        let Ok((endpoint, client)) = endpoint_query.get(trigger.entity) else {
            return;
        };
        let Ok(io) = client_query.get(client.0) else {
            return;
        };
        commands
            .spawn((AeronetLinkOf(client.0), Name::from("IrohClient")))
            .queue(endpoint.connect(io.target.clone(), io.alpn.clone()));
    }

    /// The local endpoint failed to bind or closed unexpectedly.
    fn on_closed(
        trigger: On<EndpointClosed>,
        endpoint_query: Query<&IrohClientEndpointOf>,
        mut commands: Commands,
    ) {
        if let Ok(client) = endpoint_query.get(trigger.entity) {
            commands.entity(client.0).try_insert(Unlinked {
                reason: UnlinkReason::TransportError(format!("{}", trigger.error)),
            });
        }
    }

    /// Close the local endpoint whenever the client becomes unlinked.
    fn on_unlinked(
        trigger: On<Add<Unlinked>>,
        query: Query<&IrohLinkEndpoint, With<IrohClientIo>>,
        mut commands: Commands,
    ) {
        if let Ok(endpoint) = query.get(trigger.entity) {
            commands.entity(endpoint.0).try_despawn();
            commands.entity(trigger.entity).remove::<IrohLinkEndpoint>();
        }
    }

    /// A client endpoint only dials out; reject anyone dialing in so the request is answered.
    fn reject_incoming(
        mut trigger: On<SessionRequest>,
        endpoint_query: Query<(), With<IrohClientEndpointOf>>,
    ) {
        if endpoint_query.contains(trigger.endpoint_entity) {
            trigger.respond(SessionResponse::rejected("client endpoint"));
        }
    }
}
