use std::net::{IpAddr, Ipv4Addr};
use bevy::{log::{Level, LogPlugin}, prelude::*};
use bevy_dtls::client::{
    cert_option::ClientCertOption, 
    dtls_client::{DtlsClient, DtlsClientConfig}, 
    event::DtlsClientEvent
};
use bevy_renet::{
    renet::{ConnectionConfig, DefaultChannel, RenetClient}, 
    RenetClientPlugin
};
use bevy_renet_dtls::client::{RenetClientDtlsExt, RenetDtlsClientPlugin};
use bytes::Bytes;

#[derive(Resource)]
struct ClientHellooonCounter(u64);

fn send_hellooon_system(
    mut renet_client: ResMut<RenetClient>,
    mut dtls_client: ResMut<DtlsClient>,
    mut counter: ResMut<ClientHellooonCounter>
) {
    if renet_client.is_disconnected() {
        return;
    }

    let str = format!("from client helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    renet_client.send_message(DefaultChannel::ReliableOrdered, msg);
    counter.0 += 1;

    if counter.0 % 10 == 0 {
        info!("disconnecting. will restart soon...");
        // disconnect dtls and close renet
        renet_client.disconnect_dtls(&mut dtls_client);
        counter.0 = 0;
    }
}

fn recv_hellooon_system(mut renet_client: ResMut<RenetClient>) {
    let ch_len = 3_u8;
    for ch in 0..ch_len {
        loop {
            let Some(bytes) = renet_client.receive_message(ch) else {
                break;
            };

            let msg = String::from_utf8(bytes.to_vec())
            .unwrap();
            info!("message: {msg}");
        }
    }
}

fn handle_net_event(
    mut renet_client: Option<ResMut<RenetClient>>,
    mut dtls_client: ResMut<DtlsClient>,
    mut dtls_events: EventReader<DtlsClientEvent>,
    mut restart: ResMut<Restart>
) {
    for e in dtls_events.read() {
        match e {
            DtlsClientEvent::SendTimeout { .. } => {
                error!("sending timeout")
            }
            DtlsClientEvent::Error { err } => {
                if err.to_string()
                .ends_with("Alert is Fatal or Close Notify")
                || err.to_string()
                .ends_with("conn is closed") {
                    info!("server disconneted: {err}");
                } else {
                    error!("{err:?}");
                }
            
                if let Some(ref mut renet) = renet_client {
                    renet.disconnect_dtls(&mut dtls_client);
                }
            }
            DtlsClientEvent::ConnClosed => {
                // this event can be emitted even before disconnect() is called
                // just make sure close before restart
                if let Some(ref mut renet) = renet_client {
                    renet.disconnect_dtls(&mut dtls_client);
                }

                restart.0 = true;
            }
        }
    }
}

fn handle_restart(
    mut commands: Commands,
    mut dtls_client: ResMut<DtlsClient>,
    client_config: Res<ClientConfig>,
    mut restart: ResMut<Restart>
) {
    if !restart.0 {
        return;
    }

    if !dtls_client.is_closed() {
        return;
    }

    info!("restarting...");
    // will insert new renet client
    let mut new_renet = RenetClient::new(ConnectionConfig::default());

    if let Err(e) = new_renet.start_dtls(&mut dtls_client, client_config.0.clone()) {
        warn!("{e}");
        return;
    }

    // overwrite with new client 
    commands.insert_resource(new_renet);
    restart.0 = false;
}

#[derive(Resource)]
struct Restart(bool);

#[derive(Resource)]
struct ClientConfig(DtlsClientConfig);

struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        let mut renet_client = RenetClient::new(ConnectionConfig::default());
        let mut dtls_client = app.world_mut()
        .resource_mut::<DtlsClient>();

        let client_config = ClientConfig(DtlsClientConfig{
            server_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            server_port: 44443,
            client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_port: 0,
            cert_option: ClientCertOption::Load { 
                server_name: "webrtc.rs",
                root_ca_path: "my_certificates/server.pub.pem" 
            }
        });

        if let Err(e) = renet_client.start_dtls(
            &mut dtls_client, 
            client_config.0.clone()
        ) {
            panic!("{e}");
        }

        app.insert_resource(client_config)
        .insert_resource(renet_client)
        .insert_resource(ClientHellooonCounter(0))
        .insert_resource(Restart(false))
        .add_systems(Update, (
            handle_net_event,
            handle_restart,
            send_hellooon_system
            .run_if(resource_exists::<RenetClient>),
            recv_hellooon_system
            .run_if(resource_exists::<RenetClient>)
        ).chain());

        info!("client connected");
    }
}

fn main() {
    App::new()
    .add_plugins((
        DefaultPlugins.set(LogPlugin{
            level: Level::INFO,
            ..default()
        }),
        RenetClientPlugin,
        RenetDtlsClientPlugin{
            timeout_secs: 5,
            buf_size: 1500
        }
    ))
    .add_plugins(ClientPlugin)
    .run();
}
