//! Connection promotion plugin for iroh QUIC sessions.
//!
//! `lightyear`'s networking stack has a two-stage pipeline:
//!
//! **Stage 1** — Transport-specific plugin (`lightyear_iroh`): creates aeronet
//! session entities and lightyear [`Link`]/[`LinkOf`] entities, then bridges the
//! aeronet lifecycle (`Session` added/removed) to the lightyear link states
//! (`Linking` → `Linked` → `Unlinked`).
//!
//! **Stage 2** — Connection promotion plugin (this module): observes `Linked` on
//! link entities and promotes them to the `Connected`/`Started` connection layer,
//! with proper peer identity assignment.
//!
//! # Comparison with `lightyear_raw_connection`
//!
//! `lightyear_raw_connection::RawConnectionPlugin` performs the same role for
//! UDP/WebSocket/WebTransport transports. It derives [`PeerId`] from the link's
//! socket address (`PeerId::Raw(addr)`), which is always available for those
//! transports.
//!
//! For iroh, **relay-only connections have no direct socket address**. This
//! plugin derives [`PeerId`] from the remote iroh [`EndpointId`] (public key)
//! instead, using [`PeerId::Entity`] with a stable hash of the key bytes.
//! Direct connections also use the `EndpointId`-derived identity for
//! consistency — the socket address of a direct iroh path can change as
//! paths are re-selected, while the public key is stable.
//!
//! # Usage
//!
//! Add [`IrohConnectionPlugin`] to your app alongside [`IrohClientPlugin`]
//! and/or [`IrohServerPlugin`]. This plugin automatically adds the required
//! `lightyear_connection::client::ConnectionPlugin` and
//! `lightyear_connection::server::ConnectionPlugin` if not already present.
//!
//! ```rust,ignore
//! app.add_plugins((
//!     IrohClientPlugin,
//!     IrohServerPlugin,
//!     IrohConnectionPlugin,
//! ));
//! ```

use bevy_app::{App, Plugin, PostUpdate, PreUpdate};
use bevy_ecs::prelude::*;
#[allow(unused_imports)]
use lightyear_core::id::{LocalId, PeerId, RemoteId};
use lightyear_link::prelude::*;
use lightyear_transport::plugin::TransportSystems;

// The imports below are used only inside feature-gated observer methods.
// `#[allow(unused_imports)]` prevents warnings when only a subset of features
// is enabled (e.g. `server`-only build does not need `Disconnect`).
#[allow(unused_imports)]
use alloc::string::ToString;
#[allow(unused_imports)]
use bevy_ecs::entity::UniqueEntitySlice;
#[allow(unused_imports)]
use lightyear_connection::client::{Connected, Disconnect, Disconnected, Disconnecting};
#[allow(unused_imports)]
use lightyear_connection::client_of::ClientOf;
#[allow(unused_imports)]
use lightyear_connection::host::HostClient;
#[allow(unused_imports)]
use lightyear_connection::server::{Started, Stop, Stopped, Stopping};
#[allow(unused_imports)]
use tracing::trace;

#[cfg(feature = "client")]
use crate::client::IrohClientIo;
#[cfg(feature = "server")]
use crate::server::IrohServerIo;

/// Converts an iroh [`EndpointId`](aeronet_iroh::iroh::EndpointId) to a
/// stable [`PeerId`] for use as [`LocalId`] / [`RemoteId`] in lightyear.
///
/// Uses the raw public-key bytes folded into a `u64` via XOR. This is not
/// cryptographically secure but is stable across process restarts as long as
/// the same iroh key pair is used, and is collision-resistant in practice for
/// small player counts.
pub fn endpoint_id_to_peer_id(id: aeronet_iroh::iroh::EndpointId) -> PeerId {
    // EndpointId is a public key (32 bytes). Fold the bytes into a u64 using
    // XOR so the result changes with every bit of the key.
    let bytes = id.as_bytes();
    let mut hash: u64 = 0;
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        hash ^= u64::from_le_bytes(word);
    }
    PeerId::Entity(hash)
}

/// Lightyear connection promotion plugin for iroh QUIC sessions.
///
/// See the [module-level docs](self) for a full description.
pub struct IrohConnectionPlugin;

impl IrohConnectionPlugin {
    // ── Client observers ────────────────────────────────────────────────────

    /// For iroh clients, `Linked` implies `Connected`.
    ///
    /// Derives the local [`PeerId`] from the local endpoint's [`EndpointId`]
    /// and sets [`RemoteId`] to [`PeerId::Server`].
    #[cfg(feature = "client")]
    fn on_client_linked(
        trigger: On<Add, Linked>,
        query: Query<(&IrohClientIo, Option<&aeronet_iroh::RemoteEndpointId>), Without<Connected>>,
        mut commands: Commands,
    ) {
        let Ok((io, _remote)) = query.get(trigger.entity) else {
            return;
        };
        let local_peer_id = endpoint_id_to_peer_id(io.endpoint.id());
        trace!(
            entity = ?trigger.entity,
            local_id = ?local_peer_id,
            "IrohClient Linked → Connected"
        );
        commands.entity(trigger.entity).insert((
            Connected,
            LocalId(local_peer_id),
            RemoteId(PeerId::Server),
        ));
    }

    /// For iroh clients, `Disconnect` implies `Unlink`.
    #[cfg(feature = "client")]
    fn on_client_disconnect(
        trigger: On<Disconnect>,
        query: Query<(), (Without<Disconnected>, With<IrohClientIo>)>,
        mut commands: Commands,
    ) {
        if query.get(trigger.entity).is_ok() {
            trace!(entity = ?trigger.entity, "IrohClient Disconnect → Unlink");
            commands.trigger(Unlink {
                entity: trigger.entity,
                reason: UnlinkReason::UserRequested(Some("Client requested disconnect".to_string())),
            });
        }
    }

    // ── Server observers ─────────────────────────────────────────────────────

    /// For iroh servers, `Linked` implies `Started`.
    #[cfg(feature = "server")]
    fn on_server_linked(
        trigger: On<Add, Linked>,
        query: Query<(), (With<IrohServerIo>, Without<Started>)>,
        mut commands: Commands,
    ) {
        if query.get(trigger.entity).is_ok() {
            trace!(entity = ?trigger.entity, "IrohServer Linked → Started");
            commands.entity(trigger.entity).insert(Started);
        }
    }

    /// For iroh servers, when a [`LinkOf`] entity gets `Linked` it also
    /// becomes `Connected` + [`ClientOf`].
    ///
    /// [`RemoteId`] is derived from [`aeronet_iroh::RemoteEndpointId`] on the
    /// corresponding aeronet session entity (inserted by `aeronet_iroh` when
    /// the QUIC handshake completes). The session entity is located via the
    /// `AeronetLinkOf` component on the session entity — we reverse-lookup
    /// via `AeronetLinkOf` to find which session entity points to this link.
    ///
    /// Falls back to an entity-bits-based [`PeerId`] if the identity is absent,
    /// which should not happen in practice.
    #[cfg(feature = "server")]
    fn on_link_of_linked(
        trigger: On<Add, Linked>,
        link_of_query: Query<&LinkOf, Without<Connected>>,
        server_query: Query<(), With<IrohServerIo>>,
        // Query all session entities that point to a link entity, so we can
        // reverse-look up which session corresponds to this newly linked entity.
        // aeronet_iroh inserts RemoteEndpointId on session entities;
        // lightyear_aeronet does not auto-propagate it to Link entities.
        session_query: Query<(
            Entity,
            &lightyear_aeronet::AeronetLinkOf,
            &aeronet_iroh::RemoteEndpointId,
        )>,
        mut commands: Commands,
    ) {
        let Ok(link_of) = link_of_query.get(trigger.entity) else {
            return;
        };
        if server_query.get(link_of.server).is_err() {
            return;
        }

        // Find the session entity whose AeronetLinkOf points to this link entity,
        // then read its RemoteEndpointId for a stable peer identity.
        let link_entity = trigger.entity;
        let remote_peer_id: PeerId = session_query
            .iter()
            .find(|(_, aeronet_link_of, _)| aeronet_link_of.0 == link_entity)
            .map(|(_, _, remote_id)| endpoint_id_to_peer_id(remote_id.0))
            .unwrap_or_else(|| PeerId::Entity(link_entity.to_bits()));

        trace!(
            entity = ?trigger.entity,
            ?remote_peer_id,
            "IrohServer LinkOf Linked → Connected + ClientOf"
        );
        commands.entity(trigger.entity).insert((
            Connected,
            LocalId(PeerId::Server),
            RemoteId(remote_peer_id),
            ClientOf,
        ));
    }

    /// For iroh servers, `Stop` implies `Unlink` on the server entity, and
    /// `Disconnecting` on every connected [`ClientOf`] link entity.
    #[cfg(feature = "server")]
    fn on_server_stop(
        trigger: On<Stop>,
        mut commands: Commands,
        server_query: Query<&Server, (Without<Stopped>, With<IrohServerIo>)>,
        client_query: Query<Entity, (With<ClientOf>, Without<HostClient>)>,
    ) -> Result {
        let Ok(server) = server_query.get(trigger.entity) else {
            return Ok(());
        };
        trace!(entity = ?trigger.entity, "IrohServer Stop → Unlink + disconnect clients");
        commands.trigger(Unlink {
            entity: trigger.entity,
            reason: UnlinkReason::ServerStopped,
        });
        commands.entity(trigger.entity).insert(Stopping);

        // SAFETY: the Server relationship collection contains unique entities.
        let client_entities: &[Entity] = server.collection();
        let unique_slice = unsafe { UniqueEntitySlice::from_slice_unchecked(client_entities) };
        client_query
            .iter_many_unique(unique_slice)
            .for_each(|entity| {
                commands.entity(entity).insert(Disconnecting);
            });
        Ok(())
    }
}

impl Plugin for IrohConnectionPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<lightyear_connection::client::ConnectionPlugin>() {
            app.add_plugins(lightyear_connection::client::ConnectionPlugin);
        }
        if !app.is_plugin_added::<lightyear_connection::server::ConnectionPlugin>() {
            app.add_plugins(lightyear_connection::server::ConnectionPlugin);
        }

        // Ensure the transport send/receive sets are ordered relative to link
        // send/receive so that the full stack runs in the correct order.
        app.configure_sets(
            PreUpdate,
            (LinkSystems::Receive, TransportSystems::Receive).chain(),
        );
        app.configure_sets(
            PostUpdate,
            (TransportSystems::Send, LinkSystems::Send).chain(),
        );

        #[cfg(feature = "client")]
        {
            app.add_observer(Self::on_client_linked);
            app.add_observer(Self::on_client_disconnect);
        }

        #[cfg(feature = "server")]
        {
            app.add_observer(Self::on_server_linked);
            app.add_observer(Self::on_link_of_linked);
            app.add_observer(Self::on_server_stop);
        }
    }
}
