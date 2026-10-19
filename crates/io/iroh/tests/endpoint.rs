#![cfg(all(feature = "client", feature = "server"))]

use aeronet_io::connection::PeerAddr;
use aeronet_iroh::iroh::{Endpoint as IrohEndpoint, endpoint::presets::Minimal};
use aeronet_iroh::{IrohRuntime, RemoteEndpointId};
use bevy_app::App;
use bevy_ecs::prelude::*;
use lightyear_aeronet::AeronetLinkOf;
use lightyear_connection::client::{Connected, Disconnect, Disconnected};
use lightyear_connection::client_of::ClientOf;
use lightyear_connection::server::{Started, Stop, Stopped};
use lightyear_core::id::{LocalId, PeerId, RemoteId};
use lightyear_iroh::client::{IrohClientIo, IrohClientPlugin};
use lightyear_iroh::server::{IrohServerIo, IrohServerPlugin, SessionRequest, SessionResponse};
use lightyear_iroh::{IrohConnectionPlugin, endpoint_id_to_peer_id};
use lightyear_link::prelude::*;
use std::time::{Duration, Instant};

const ALPN: &[u8] = b"lightyear/merge-test";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn endpoint(runtime: &tokio::runtime::Runtime) -> IrohEndpoint {
    runtime.block_on(async {
        IrohEndpoint::builder(Minimal)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
            .unwrap()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap()
    })
}

#[track_caller]
fn pump_until(app: &mut App, mut condition: impl FnMut(&mut World) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.update();
        if condition(app.world_mut()) {
            return;
        }
        assert!(Instant::now() < deadline, "iroh lifecycle timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn promotes_endpoint_links_with_and_without_socket_addresses() {
    let runtime = runtime();
    let endpoint = endpoint(&runtime);
    let mut app = App::new();
    app.add_plugins(IrohConnectionPlugin);
    let server = app
        .world_mut()
        .spawn(IrohServerIo {
            endpoint: endpoint.clone(),
        })
        .id();
    assert!(app.world().get::<Server>(server).is_some());
    assert!(app.world().get::<Endpoint>(server).is_some());
    app.world_mut().entity_mut(server).insert(Linked);
    assert!(app.world().get::<Started>(server).is_some());

    for peer_addr in [None, Some("127.0.0.1:5000".parse().unwrap())] {
        let link = app.world_mut().spawn(LinkOf { endpoint: server }).id();
        if let Some(addr) = peer_addr {
            app.world_mut().entity_mut(link).insert(PeerAddr(addr));
        }
        let remote =
            aeronet_iroh::iroh::SecretKey::from([if peer_addr.is_some() { 1 } else { 2 }; 32])
                .public();
        app.world_mut()
            .spawn((AeronetLinkOf(link), RemoteEndpointId(remote)));
        app.world_mut().entity_mut(link).insert(Linked);
        assert!(app.world().get::<Connected>(link).is_some());
        assert!(app.world().get::<ClientOf>(link).is_some());
        assert_eq!(
            app.world().get::<RemoteId>(link),
            Some(&RemoteId(endpoint_id_to_peer_id(remote)))
        );
        assert_eq!(
            app.world().get::<LocalId>(link),
            Some(&LocalId(PeerId::Server))
        );
    }
    runtime.block_on(endpoint.close());
}

#[test]
fn stop_tolerates_a_client_despawned_by_another_observer() {
    let runtime = runtime();
    let endpoint = endpoint(&runtime);
    let mut app = App::new();
    app.add_observer(
        |_: On<Stop>, clients: Query<Entity, With<ClientOf>>, mut commands: Commands| {
            for entity in &clients {
                commands.entity(entity).despawn();
            }
        },
    );
    app.add_plugins(IrohConnectionPlugin);
    let server = app
        .world_mut()
        .spawn((
            IrohServerIo {
                endpoint: endpoint.clone(),
            },
            Started,
        ))
        .id();
    let client = app
        .world_mut()
        .spawn((LinkOf { endpoint: server }, ClientOf))
        .id();
    app.world_mut().trigger(Stop { entity: server });
    app.world_mut().flush();
    assert!(app.world().get_entity(client).is_err());
    runtime.block_on(endpoint.close());
}

#[test]
fn local_sessions_exchange_payloads_disconnect_and_stop() {
    let runtime = runtime();
    let server_endpoint = endpoint(&runtime);
    let client_endpoint = endpoint(&runtime);
    let mut app = App::new();
    app.insert_resource(IrohRuntime::from(runtime.handle().clone()));
    app.add_plugins((IrohClientPlugin, IrohServerPlugin, IrohConnectionPlugin));
    app.add_observer(|mut request: On<SessionRequest>| request.respond(SessionResponse::Accepted));
    let server = app
        .world_mut()
        .spawn(IrohServerIo {
            endpoint: server_endpoint.clone(),
        })
        .id();
    app.world_mut().trigger(LinkStart { entity: server });
    pump_until(&mut app, |world| world.get::<Started>(server).is_some());
    let client = app
        .world_mut()
        .spawn(IrohClientIo::from_addr(
            client_endpoint.clone(),
            server_endpoint.addr(),
            ALPN.to_vec(),
        ))
        .id();
    app.world_mut().trigger(LinkStart { entity: client });
    pump_until(&mut app, |world| {
        world.get::<Connected>(client).is_some()
            && world
                .query_filtered::<Entity, With<ClientOf>>()
                .iter(world)
                .count()
                == 1
    });
    let accepted = app
        .world_mut()
        .query_filtered::<Entity, With<ClientOf>>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        app.world().get::<LinkOf>(accepted).unwrap().endpoint,
        server
    );
    assert_eq!(
        app.world().get::<RemoteId>(accepted),
        Some(&RemoteId(endpoint_id_to_peer_id(client_endpoint.id())))
    );
    app.world_mut()
        .get_mut::<Link>(client)
        .unwrap()
        .send(b"client to server".as_slice().into());
    app.world_mut()
        .get_mut::<Link>(accepted)
        .unwrap()
        .send(b"server to client".as_slice().into());
    let mut client_received = false;
    let mut server_received = false;
    pump_until(&mut app, |world| {
        client_received |= world
            .get_mut::<Link>(client)
            .unwrap()
            .recv
            .drain()
            .any(|payload| payload.as_ref() == b"server to client");
        server_received |= world
            .get_mut::<Link>(accepted)
            .unwrap()
            .recv
            .drain()
            .any(|payload| payload.as_ref() == b"client to server");
        client_received && server_received
    });
    app.world_mut().trigger(Disconnect { entity: client });
    pump_until(&mut app, |world| {
        world.get::<Disconnected>(client).is_some()
    });
    app.world_mut().trigger(Stop { entity: server });
    pump_until(&mut app, |world| {
        world.get::<Stopped>(server).is_some() && world.get_entity(accepted).is_err()
    });
    drop(app);
    runtime.block_on(async {
        client_endpoint.close().await;
        server_endpoint.close().await;
    });
}
