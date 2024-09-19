use std::net::{IpAddr, Ipv4Addr};
use bevy::{log::{Level, LogPlugin}, prelude::*};
use bevy_dtls::client::{
    cert_option::ClientCertOption, 
    dtls_client::{DtlsClient, DtlsClientConfig}, 
    health::DtlsClientError
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
        warn!("disconnecting");
        renet_client.disconnect_with_dtls(&mut dtls_client);
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

fn handle_net_error(
    mut renet_client: ResMut<RenetClient>,
    mut dtls_client: ResMut<DtlsClient>,
    mut errors: EventReader<DtlsClientError>
) {
    for e in errors.read() {
        match e {
            DtlsClientError::SendTimeout { .. } => error!("{e:?}"),
            DtlsClientError::Fatal { err } => {
                error!("{err}: disconnecting");

                renet_client.disconnect_with_dtls(&mut dtls_client);
            }
        }
    }
}

struct ClientPlugin {
    server_addr: IpAddr,
    server_port: u16,
    client_addr: IpAddr,
    client_port: u16,
    cert_option: ClientCertOption
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        let mut renet_client = RenetClient::new(ConnectionConfig::default());
        let mut dtls_client = app.world_mut()
        .resource_mut::<DtlsClient>();

        if let Err(e) = renet_client.start_with_dtls(
            &mut dtls_client,
            DtlsClientConfig{
                server_addr: self.server_addr,
                server_port: self.server_port,
                client_addr: self.client_addr,
                client_port: self.client_port,
                cert_option: self.cert_option,
            }
        ) {
            panic!("{e}");
        }

        app.insert_resource(renet_client)
        .insert_resource(ClientHellooonCounter(0))
        .add_systems(Update, (
            handle_net_error,
            send_hellooon_system,
            recv_hellooon_system
        ).chain());

        info!("client connected");
    }
}

fn main() {
    App::new()
    .add_plugins((
        DefaultPlugins.set(LogPlugin{
            level: Level::DEBUG,
            ..default()
        }),
        RenetClientPlugin,
        RenetDtlsClientPlugin{
            timeout_secs: 10,
            buf_size: 1500
        }
    ))
    .add_plugins(
        ClientPlugin{
            server_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            server_port: 4443,
            client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_port: 0,
            cert_option: ClientCertOption::Load { 
                server_name: "webrtc.rs",
                root_ca_path: "my_certificates/server.pub.pem" 
            }
        }
    )
    .run();
}
