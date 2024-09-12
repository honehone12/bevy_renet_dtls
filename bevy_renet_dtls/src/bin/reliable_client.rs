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
use bevy_renet_dtls::client::{DtlsClientRenetExt, RenetDtlsClientPlugin};
use bytes::Bytes;

#[derive(Resource)]
struct ClientHellooonCounter(usize);

fn send_hellooon_system(
    mut renet_client: ResMut<RenetClient>,
    mut counter: ResMut<ClientHellooonCounter>
) {
    let str = format!("from client helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    renet_client.send_message(DefaultChannel::ReliableOrdered, msg);
    counter.0 += 1;
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

fn handle_net_error(mut errors: EventReader<DtlsClientError>) {
    for e in errors.read() {
        error!("{e:?}");
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

        if let Err(e) = dtls_client.start_renet_dtls(
            DtlsClientConfig{
                server_addr: self.server_addr,
                server_port: self.server_port,
                client_addr: self.client_addr,
                client_port: self.client_port,
                cert_option: self.cert_option,
            },
            &mut renet_client
        ) {
            panic!("{e}");
        }

        app.insert_resource(renet_client)
        .insert_resource(ClientHellooonCounter(0))
        .add_systems(Update, (
            handle_net_error,
            recv_hellooon_system,
            send_hellooon_system
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
            timeout_secs: 10,
            buf_size: 512
        }
    ))
    .add_plugins(
        ClientPlugin{
            server_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            server_port: 4443,
            client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_port: 0,
            cert_option: ClientCertOption::GenerateSelfSigned { 
                subject_alt_name: "webrtc.rs" 
            },
        }
    )
    .run();
}
